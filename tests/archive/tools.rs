//! Tool-logic tests: the owner default, unknown-type rendering, deletion hiding, that an
//! unassigned weigh-in stays out of the owner's trend, and the `sql` tool's three guards
//! (no write, no session table, no runaway query).

use std::time::Duration;

use chrono::{TimeZone, Utc};
use serde_json::json;
use withings_core::db::{create_pool, ensure_schema};
use withings_core::models::{Measure, MeasureGroup, Person};
use withings_core::store::{measures, people, sql};
use withings_mcp_server::tools;

use crate::support::{self, db_test};

async fn owner_with_weighin(
    pool: &sqlx::PgPool,
    grpid: i64,
    type_code: i32,
    value: i64,
    unit: i32,
) {
    let mut conn = pool.acquire().await.unwrap();
    people::upsert(
        &mut conn,
        &Person {
            id: 1,
            label: Some("ALPHA".into()),
            is_owner: true,
            shared: false,
            raw: json!({}),
        },
    )
    .await
    .unwrap();
    let g = MeasureGroup {
        grpid,
        person_id: Some(1),
        ts: Utc.with_ymd_and_hms(2026, 3, 1, 8, 0, 0).unwrap(),
        device_id: None,
        model: None,
        category: Some(1),
        attrib: Some(0),
        modified_at: None,
        measures: vec![Measure {
            type_code,
            value,
            unit,
            algo: None,
            fm: None,
        }],
        raw: json!({}),
    };
    measures::upsert_group(&mut conn, &g).await.unwrap();
}

db_test!(measurements_default_to_the_owners_latest, |f| {
    owner_with_weighin(&f.pool, 1, 1, 7234, -2).await;
    let out = tools::measurements(&f.pool, None, None, None, &[], false, None)
        .await
        .unwrap();
    assert!(out.contains("weight"), "{out}");
    assert!(out.contains("72.34"), "{out}");
});

db_test!(unknown_types_render_by_code, |f| {
    owner_with_weighin(&f.pool, 1, 9999, 5, 0).await;
    let out = tools::measurements(&f.pool, None, None, None, &[], false, None)
        .await
        .unwrap();
    assert!(out.contains("type 9999"), "{out}");
});

db_test!(deleted_groups_are_hidden_unless_asked, |f| {
    owner_with_weighin(&f.pool, 7, 1, 7000, -2).await;
    let mut conn = f.pool.acquire().await.unwrap();
    measures::mark_deleted(&mut conn, 7).await.unwrap();
    // Hidden by default.
    let hidden = tools::measurements(&f.pool, None, None, None, &[], false, None)
        .await
        .unwrap();
    assert!(hidden.contains("No measurements"), "{hidden}");
    // Shown when asked.
    let shown = tools::measurements(&f.pool, None, None, None, &[], true, None)
        .await
        .unwrap();
    assert!(shown.contains("deleted"), "{shown}");
});

db_test!(unassigned_weigh_ins_stay_out_of_the_owners_trend, |f| {
    owner_with_weighin(&f.pool, 1, 1, 7000, -2).await;
    // A weigh-in with no person (the scale could not attribute it).
    let mut conn = f.pool.acquire().await.unwrap();
    let orphan = MeasureGroup {
        grpid: 2,
        person_id: None,
        ts: Utc.with_ymd_and_hms(2026, 3, 1, 9, 0, 0).unwrap(),
        device_id: None,
        model: None,
        category: Some(1),
        attrib: Some(0),
        modified_at: None,
        measures: vec![Measure {
            type_code: 1,
            value: 9999,
            unit: -2,
            algo: None,
            fm: None,
        }],
        raw: json!({}),
    };
    measures::upsert_group(&mut conn, &orphan).await.unwrap();
    let out = tools::trend(&f.pool, "weight", None, None, None, Some("day"), "UTC")
        .await
        .unwrap();
    // The owner's 70.00 shows; the orphan's 99.99 does not.
    assert!(out.contains("70"), "{out}");
    assert!(!out.contains("99.99"), "{out}");
});

db_test!(sql_refuses_a_write, |f| {
    // A write is refused twice over: it cannot be a subquery, and the transaction is read
    // only. Either way it is a usage error and nothing is written.
    let out = tools::sql_tool(&f.pool, "INSERT INTO person (id) VALUES (99)")
        .await
        .unwrap_err();
    assert!(matches!(out, tools::ToolError::Usage(_)), "{out:?}");
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM person WHERE id = 99")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
});

db_test!(sql_times_out, |f| {
    // run_readonly directly with a short timeout, so the test is fast: pg_sleep outlives it.
    let err = sql::run_readonly(
        &f.pool,
        "SELECT pg_sleep(5)",
        500,
        Duration::from_millis(500),
    )
    .await
    .unwrap_err();
    // Postgres cancels on statement_timeout (SQLSTATE 57014).
    assert_eq!(
        err.as_database_error().and_then(|e| e.code()).as_deref(),
        Some("57014"),
        "{err}"
    );
});

const SESSION_DB: &str = "withings_mcp_sql_session";

db_test!(sql_cannot_read_the_session, |f| {
    let Some(dsn) = support::container_dsn().await else {
        return;
    };
    // The production role, with LOGIN so the sql tool can run as it. 0001 revokes
    // `ingest_session` from exactly this role, so the grant below reaches every other
    // table and not the session jar.
    sqlx::query(
        "DO $$ BEGIN CREATE ROLE withings_mcp LOGIN PASSWORD 'pw'; \
         EXCEPTION WHEN duplicate_object THEN NULL; END $$",
    )
    .execute(&f.pool)
    .await
    .unwrap();
    sqlx::query("ALTER ROLE withings_mcp LOGIN PASSWORD 'pw'")
        .execute(&f.pool)
        .await
        .unwrap();
    sqlx::query("DROP DATABASE IF EXISTS withings_mcp_sql_session WITH (FORCE)")
        .execute(&f.pool)
        .await
        .unwrap();
    sqlx::query("CREATE DATABASE withings_mcp_sql_session")
        .execute(&f.pool)
        .await
        .unwrap();
    let (base, _) = dsn.rsplit_once('/').expect("the dsn names a database");
    let admin = create_pool(&format!("{base}/{SESSION_DB}")).await.unwrap();
    sqlx::query("GRANT USAGE ON SCHEMA public TO withings_mcp")
        .execute(&admin)
        .await
        .unwrap();
    sqlx::query("ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT SELECT ON TABLES TO withings_mcp")
        .execute(&admin)
        .await
        .unwrap();
    // The session table is revoked by 0001 from withings_mcp.
    ensure_schema(&admin).await.unwrap();

    let mcp_dsn = {
        let at = dsn.rsplit_once('@').unwrap().1;
        let host_db = at.rsplit_once('/').unwrap().0;
        format!("postgresql://withings_mcp:pw@{host_db}/{SESSION_DB}")
    };
    let as_mcp = create_pool(&mcp_dsn).await.unwrap();
    let denied = sql::run_readonly(
        &as_mcp,
        "SELECT count(*) FROM ingest_session",
        10,
        Duration::from_secs(5),
    )
    .await
    .unwrap_err();
    assert_eq!(
        denied.as_database_error().and_then(|e| e.code()).as_deref(),
        Some("42501"),
        "{denied}"
    );
    as_mcp.close().await;
    admin.close().await;
    sqlx::query("DROP DATABASE IF EXISTS withings_mcp_sql_session WITH (FORCE)")
        .execute(&f.pool)
        .await
        .unwrap();
});
