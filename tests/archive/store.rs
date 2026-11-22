//! Store tests: group upsert with revisions and deletion, the budget's UTC-day reset,
//! local-time trend bucketing, and the owner default.

use chrono::{TimeZone, Utc};
use serde_json::json;
use withings_core::models::{Bucket, GroupChange, Measure, MeasureGroup, Person};
use withings_core::store::{measures, people, sync};

use crate::support::db_test;

fn owner() -> Person {
    Person {
        id: 1,
        label: Some("ALPHA".into()),
        is_owner: true,
        shared: false,
        raw: json!({}),
    }
}

fn group(grpid: i64, measures: Vec<Measure>) -> MeasureGroup {
    MeasureGroup {
        grpid,
        person_id: Some(1),
        ts: Utc.with_ymd_and_hms(2026, 3, 1, 8, 0, 0).unwrap(),
        device_id: Some("dev".into()),
        model: Some(5),
        category: Some(1),
        attrib: Some(0),
        modified_at: None,
        measures,
        raw: json!({"grpid": grpid}),
    }
}

fn measure(type_code: i32, value: i64, unit: i32) -> Measure {
    Measure {
        type_code,
        value,
        unit,
        algo: None,
        fm: None,
    }
}

db_test!(a_new_group_is_new_and_a_second_write_is_unchanged, |f| {
    let mut conn = f.pool.acquire().await.unwrap();
    people::upsert(&mut conn, &owner()).await.unwrap();
    let g = group(100, vec![measure(1, 7234, -2)]);
    assert_eq!(
        measures::upsert_group(&mut conn, &g).await.unwrap(),
        GroupChange::New
    );
    assert_eq!(
        measures::upsert_group(&mut conn, &g).await.unwrap(),
        GroupChange::Unchanged
    );
});

db_test!(a_changed_value_is_revised_and_the_old_value_kept, |f| {
    let mut conn = f.pool.acquire().await.unwrap();
    people::upsert(&mut conn, &owner()).await.unwrap();
    measures::upsert_group(&mut conn, &group(200, vec![measure(1, 7234, -2)]))
        .await
        .unwrap();
    // The same group, weight corrected to 72.00.
    let change = measures::upsert_group(&mut conn, &group(200, vec![measure(1, 7200, -2)]))
        .await
        .unwrap();
    assert_eq!(change, GroupChange::Revised);
    let old: i64 = sqlx::query_scalar(
        "SELECT value FROM measure_revision WHERE grpid = 200 AND type_code = 1",
    )
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(old, 7234);
    let now: i64 =
        sqlx::query_scalar("SELECT value FROM measure WHERE grpid = 200 AND type_code = 1")
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(now, 7200);
});

db_test!(a_deleted_group_is_marked_and_comes_back_clean, |f| {
    let mut conn = f.pool.acquire().await.unwrap();
    people::upsert(&mut conn, &owner()).await.unwrap();
    let g = group(300, vec![measure(1, 7000, -2)]);
    measures::upsert_group(&mut conn, &g).await.unwrap();
    // The getmeas `deleted` flag path: the poll marks the group deleted.
    measures::mark_deleted(&mut conn, 300).await.unwrap();
    let deleted: bool =
        sqlx::query_scalar("SELECT deleted_at IS NOT NULL FROM measure_group WHERE grpid = 300")
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert!(deleted);
    // It reappears: deleted_at is cleared.
    measures::upsert_group(&mut conn, &g).await.unwrap();
    let still: bool =
        sqlx::query_scalar("SELECT deleted_at IS NOT NULL FROM measure_group WHERE grpid = 300")
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert!(!still);
});

db_test!(the_budget_resets_on_a_new_utc_day, |f| {
    // Spend 3 of a ceiling of 5.
    assert!(sync::take_budget(&f.pool, "backfill", 3, 5).await.unwrap());
    // 3 more would exceed 5: refused.
    assert!(!sync::take_budget(&f.pool, "backfill", 3, 5).await.unwrap());
    // Force yesterday, then a fresh request gets the full allowance again.
    sqlx::query("UPDATE sync_state SET budget_day = current_date - 1 WHERE loop = 'backfill'")
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(sync::take_budget(&f.pool, "backfill", 3, 5).await.unwrap());
    let spent: i64 =
        sqlx::query_scalar("SELECT budget_spent FROM sync_state WHERE loop = 'backfill'")
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(spent, 3);
});

db_test!(trend_buckets_by_local_day, |f| {
    let mut conn = f.pool.acquire().await.unwrap();
    people::upsert(&mut conn, &owner()).await.unwrap();
    // 2026-03-01 23:30 UTC is 2026-03-02 00:30 in Warsaw: the trend's day bucket is the 2nd.
    let mut g = group(400, vec![measure(1, 7000, -2)]);
    g.ts = Utc.with_ymd_and_hms(2026, 3, 1, 23, 30, 0).unwrap();
    measures::upsert_group(&mut conn, &g).await.unwrap();
    let rows = measures::trend(&f.pool, None, 1, None, None, Bucket::Day, "Europe/Warsaw")
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    // The bucket start is 2026-03-02 00:00 Warsaw = 2026-03-01 23:00 UTC.
    assert_eq!(
        rows[0].bucket,
        Utc.with_ymd_and_hms(2026, 3, 1, 23, 0, 0).unwrap()
    );
    assert_eq!(rows[0].count, 1);
});

db_test!(none_person_means_the_owner, |f| {
    let mut conn = f.pool.acquire().await.unwrap();
    people::upsert(&mut conn, &owner()).await.unwrap();
    // A second, non-owner profile with its own weigh-in.
    people::upsert(
        &mut conn,
        &Person {
            id: 2,
            label: Some("BRAVO".into()),
            is_owner: false,
            shared: true,
            raw: json!({}),
        },
    )
    .await
    .unwrap();
    let mut g1 = group(500, vec![measure(1, 7000, -2)]);
    g1.person_id = Some(1);
    measures::upsert_group(&mut conn, &g1).await.unwrap();
    let mut g2 = group(501, vec![measure(1, 9000, -2)]);
    g2.person_id = Some(2);
    measures::upsert_group(&mut conn, &g2).await.unwrap();
    // person = None resolves to the owner (id 1), so only grp 500 shows.
    let rows = measures::series(&f.pool, None, &[], None, None, false, 20)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].grpid, 500);
});
