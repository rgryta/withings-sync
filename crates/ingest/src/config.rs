//! Ingester configuration from the environment.

use std::time::Duration;

use withings_core::config::Secret;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("{0} is required")]
    Missing(&'static str),
    #[error("{name} is not a number: {value:?}")]
    NotANumber { name: &'static str, value: String },
}

#[derive(Debug, Clone)]
pub struct IngestSettings {
    pub dsn: Secret,
    /// The account label, used as `account.id` / `ingest_session.account_id`.
    pub account: String,
    pub poll_interval: Duration,
    pub auth_retry: Duration,
    /// Requests per UTC day the backfill may spend.
    pub backfill_budget: i64,
    /// First start only: a path to a seed session JSON. Ignored once the DB holds one.
    pub session_from: Option<String>,
}

fn secs(name: &'static str, default: u64) -> Result<Duration, ConfigError> {
    match std::env::var(name) {
        Ok(raw) => {
            let n: u64 = raw
                .trim()
                .parse()
                .map_err(|_| ConfigError::NotANumber { name, value: raw })?;
            Ok(Duration::from_secs(n))
        }
        Err(_) => Ok(Duration::from_secs(default)),
    }
}

impl IngestSettings {
    pub fn from_env() -> Result<Self, ConfigError> {
        let dsn =
            std::env::var("WITHINGS_DSN").map_err(|_| ConfigError::Missing("WITHINGS_DSN"))?;
        let account = std::env::var("WITHINGS_ACCOUNT")
            .map_err(|_| ConfigError::Missing("WITHINGS_ACCOUNT"))?;
        let backfill_budget = match std::env::var("WITHINGS_BACKFILL_BUDGET") {
            Ok(raw) => raw.trim().parse().map_err(|_| ConfigError::NotANumber {
                name: "WITHINGS_BACKFILL_BUDGET",
                value: raw,
            })?,
            Err(_) => 500,
        };
        Ok(Self {
            dsn: Secret::new(dsn),
            account,
            poll_interval: secs("WITHINGS_POLL_INTERVAL", 20 * 60)?,
            auth_retry: secs("WITHINGS_AUTH_RETRY", 6 * 60 * 60)?,
            backfill_budget,
            session_from: std::env::var("WITHINGS_SESSION_FROM")
                .ok()
                .filter(|s| !s.is_empty()),
        })
    }
}
