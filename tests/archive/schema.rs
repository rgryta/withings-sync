//! Schema tests: the migrations apply, the measure view scales by unit and keeps unknown
//! codes, and the MCP role cannot read the session jar.

use withings_core::db::{create_pool, ensure_schema};

use crate::support::{self, db_test};

db_test!(migrations_apply_twice_cleanly, |f| {
    // The fixture already applied the schema once (container_dsn does it). A second pass
    // applies nothing and does not fail.
    let again = ensure_schema(&f.pool).await.expect("re-apply is a no-op");
    assert!(again.is_empty(), "second apply ran: {again:?}");
});

db_test!(measure_value_scales_by_unit, |f| {
    sqlx::query("INSERT INTO person (id, is_owner) VALUES (1, true)")
        .execute(&f.pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO measure_group (grpid, person_id, ts, category, raw) \
         VALUES (10, 1, now(), 1, '{}'::jsonb)",
    )
    .execute(&f.pool)
    .await
    .unwrap();
    // 7234 * 10^-2 = 72.34 kg.
    sqlx::query("INSERT INTO measure (grpid, type_code, value, unit) VALUES (10, 1, 7234, -2)")
        .execute(&f.pool)
        .await
        .unwrap();
    let value: String = sqlx::query_scalar(
        "SELECT value::text FROM measure_value WHERE grpid = 10 AND type_code = 1",
    )
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(value, "72.34");
    let name: Option<String> = sqlx::query_scalar(
        "SELECT type_name FROM measure_value WHERE grpid = 10 AND type_code = 1",
    )
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(name.as_deref(), Some("weight"));
});

db_test!(unknown_codes_survive_the_view, |f| {
    sqlx::query("INSERT INTO person (id, is_owner) VALUES (2, true)")
        .execute(&f.pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO measure_group (grpid, person_id, ts, category, raw) \
         VALUES (20, 2, now(), 1, '{}'::jsonb)",
    )
    .execute(&f.pool)
    .await
    .unwrap();
    // A code the seed table has never heard of: stored, and present in the view with a
    // null name rather than dropped by the join.
    sqlx::query("INSERT INTO measure (grpid, type_code, value, unit) VALUES (20, 9999, 5, 0)")
        .execute(&f.pool)
        .await
        .unwrap();
    let row: (Option<String>, String) = sqlx::query_as(
        "SELECT type_name, value::text FROM measure_value WHERE grpid = 20 AND type_code = 9999",
    )
    .fetch_one(&f.pool)
    .await
    .expect("the unknown code is in the view");
    assert_eq!(row.0, None);
    assert_eq!(row.1, "5");
});

const GRANTS_DB: &str = "withings_session_grants";

db_test!(the_mcp_role_cannot_read_the_session_jar, |f| {
    let Some(dsn) = support::container_dsn().await else {
        return;
    };
    sqlx::query(
        "DO $$ BEGIN CREATE ROLE withings_mcp; \
         EXCEPTION WHEN duplicate_object THEN NULL; END $$",
    )
    .execute(&f.pool)
    .await
    .unwrap();
    sqlx::query("DROP DATABASE IF EXISTS withings_session_grants WITH (FORCE)")
        .execute(&f.pool)
        .await
        .unwrap();
    sqlx::query("CREATE DATABASE withings_session_grants")
        .execute(&f.pool)
        .await
        .unwrap();
    let (base, _) = dsn.rsplit_once('/').expect("the dsn names a database");
    let pool = create_pool(&format!("{base}/{GRANTS_DB}")).await.unwrap();
    sqlx::query("GRANT USAGE ON SCHEMA public TO withings_mcp")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT SELECT ON TABLES TO withings_mcp")
        .execute(&pool)
        .await
        .unwrap();
    ensure_schema(&pool).await.unwrap();

    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET LOCAL ROLE withings_mcp")
        .execute(&mut *tx)
        .await
        .unwrap();
    let groups: i64 = sqlx::query_scalar("SELECT count(*) FROM measure_group")
        .fetch_one(&mut *tx)
        .await
        .expect("the default grant reaches the archive");
    assert_eq!(groups, 0);
    let denied = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM ingest_session")
        .fetch_one(&mut *tx)
        .await
        .expect_err("the MCP role must not read the session jar");
    assert_eq!(
        denied.as_database_error().and_then(|e| e.code()).as_deref(),
        Some("42501"),
        "{denied}"
    );
    drop(tx);
    pool.close().await;
    sqlx::query("DROP DATABASE IF EXISTS withings_session_grants WITH (FORCE)")
        .execute(&f.pool)
        .await
        .unwrap();
});

db_test!(the_body_composition_percent_codes_are_labelled, |f| {
    // 0003 names the percentage codes and corrects the two 0001 got wrong.
    let names: Vec<(i32, String)> =
        sqlx::query_as("SELECT code, name FROM measure_type WHERE code = ANY($1) ORDER BY code")
            .bind(vec![93, 94, 95, 122, 170])
            .fetch_all(&f.pool)
            .await
            .unwrap();
    assert_eq!(
        names,
        vec![
            (93, "muscle_percent".to_string()),
            (94, "bone_percent".to_string()),
            (95, "water_percent".to_string()),
            (122, "lean_mass_percent".to_string()),
            (170, "visceral_fat".to_string()),
        ]
    );
});
