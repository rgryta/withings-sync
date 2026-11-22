//! Test fixtures.
//!
//! Postgres tests run against a throwaway testcontainer and report a loud skip when
//! Docker is unreachable. They must never reach the homelab database: **nothing here
//! reads a DSN from the environment**, the container builds its own.

#![allow(dead_code)]

use sqlx::postgres::PgPool;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use testcontainers_modules::testcontainers::{ContainerAsync, ImageExt, ReuseDirective};
use tokio::sync::{Mutex, OnceCell};
use withings_core::db::{create_pool, ensure_schema};

/// Every table the migrations create, minus `schema_migration`. Extended as the schema
/// grows; `measure` and `measure_revision` go with `measure_group` through the CASCADE.
const TRUNCATE_ALL: &str = "TRUNCATE measure_group, person, device, measure_type, \
                            ingest_session, sync_state, account CASCADE";

/// One named container serves every run, parked in a `static` and marked for reuse.
/// Remove it with `docker rm -f withings-sync-test-pg` when you want the disk back.
const CONTAINER_NAME: &str = "withings-sync-test-pg";

static CONTAINER: OnceCell<Option<(ContainerAsync<Postgres>, String)>> = OnceCell::const_new();
static TURN: Mutex<()> = Mutex::const_new(());

pub struct Fixture {
    pub pool: PgPool,
    _turn: tokio::sync::MutexGuard<'static, ()>,
}

pub async fn fixture() -> Option<Fixture> {
    let turn = TURN.lock().await;
    let dsn = container_dsn().await?;
    let pool = create_pool(&dsn).await.expect("the container is up");
    sqlx::query(TRUNCATE_ALL)
        .execute(&pool)
        .await
        .expect("truncate");
    Some(Fixture { pool, _turn: turn })
}

pub async fn container_dsn() -> Option<String> {
    CONTAINER
        .get_or_init(|| async {
            let request = Postgres::default()
                .with_tag("18-alpine")
                .with_container_name(CONTAINER_NAME)
                .with_reuse(ReuseDirective::Always);
            match request.start().await {
                Ok(container) => {
                    let host = container.get_host().await.ok()?;
                    let port = container.get_host_port_ipv4(5432).await.ok()?;
                    let dsn = format!("postgresql://postgres:postgres@{host}:{port}/postgres");
                    let pool = create_pool(&dsn).await.ok()?;
                    ensure_schema(&pool).await.ok()?;
                    pool.close().await;
                    Some((container, dsn))
                }
                Err(error) => {
                    eprintln!("SKIPPING the postgres tests: docker is not reachable: {error}");
                    None
                }
            }
        })
        .await
        .as_ref()
        .map(|(_, dsn)| dsn.clone())
}

#[allow(unused_macros)]
macro_rules! db_test {
    ($name:ident, |$fixture:ident| $body:block) => {
        #[tokio::test]
        async fn $name() {
            let Some($fixture) = crate::support::fixture().await else {
                return;
            };
            $body
        }
    };
}
#[allow(unused_imports)]
pub(crate) use db_test;
