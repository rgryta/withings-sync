//! The ingest daemon: `withings-ingest`.
//!
//! It never logs in. It refreshes the seeded session, backfills the whole history once
//! under a daily budget, then polls for changes. A rejected session is recorded and
//! waited on; the process does not exit, so a dead session cannot become a restart storm.

use std::process::ExitCode;

use withings_client::Client;
use withings_core::db::{create_pool, ensure_schema};
use withings_core::store::sync as sync_store;
use withings_ingest::config::IngestSettings;
use withings_ingest::runner::{LOOP, RunError, Runner, load_session};

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "withings_ingest=info,info".into()),
        )
        .init();

    let settings = match IngestSettings::from_env() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("configuration error: {e}");
            return ExitCode::FAILURE;
        }
    };

    let pool = match create_pool(settings.dsn.expose()).await {
        Ok(p) => p,
        Err(e) => {
            eprintln!("cannot connect to the database: {e}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(e) = ensure_schema(&pool).await {
        eprintln!("cannot apply the schema: {e}");
        return ExitCode::FAILURE;
    }

    loop {
        let session = match load_session(&pool, &settings.account, settings.session_from.as_deref())
            .await
        {
            Ok(Some(s)) => s,
            Ok(None) => {
                tracing::warn!("no session: seed one, then I will pick it up");
                let _ =
                    sync_store::record_auth_error(&pool, &settings.account, "no session seeded")
                        .await;
                tokio::time::sleep(settings.auth_retry).await;
                continue;
            }
            Err(e) => {
                tracing::error!(%e, "cannot load the session");
                tokio::time::sleep(settings.poll_interval).await;
                continue;
            }
        };

        let mut runner = Runner::new(
            pool.clone(),
            settings.account.clone(),
            settings.backfill_budget,
            Client::new(),
            session,
        );

        match run(&mut runner, &pool, &settings).await {
            RunError::Rejected(code) => {
                tracing::error!(code, "session rejected; waiting for a fresh seed");
                let _ = sync_store::record_auth_error(
                    &pool,
                    &settings.account,
                    &format!("session rejected (status {code})"),
                )
                .await;
                tokio::time::sleep(settings.auth_retry).await;
            }
            other => {
                tracing::error!(%other, "pass failed; retrying after the poll interval");
                let _ = sync_store::record_error(&pool, LOOP, &other.to_string()).await;
                tokio::time::sleep(settings.poll_interval).await;
            }
        }
    }
}

/// Run until an error bubbles up (the caller decides how long to wait on it). Backfill to
/// completion, then poll forever.
async fn run(
    runner: &mut Runner<Client>,
    pool: &sqlx::postgres::PgPool,
    settings: &IngestSettings,
) -> RunError {
    loop {
        // Backfill one step at a time until done, pausing when the budget is spent.
        match runner.backfill_step().await {
            Ok(true) => {}
            Ok(false) => {
                // More history, or the budget is spent for today: pause, then continue.
                let _ = sync_store::clear_auth_error(pool, &settings.account).await;
                tokio::time::sleep(settings.poll_interval).await;
                continue;
            }
            Err(e) => return e,
        }
        match runner.pass().await {
            Ok(report) => {
                let _ = sync_store::clear_auth_error(pool, &settings.account).await;
                tracing::info!(
                    groups = report.groups,
                    revised = report.revised,
                    deleted = report.deleted,
                    scores = report.scores,
                    "pass complete"
                );
            }
            Err(e) => return e,
        }
        tokio::time::sleep(settings.poll_interval).await;
    }
}
