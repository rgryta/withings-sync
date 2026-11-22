//! Measurement groups and values: upsert with revision history, deletion marking, and
//! the read side the MCP serves.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use sqlx::AssertSqlSafe;
use sqlx::{PgConnection, Row, postgres::PgPool};

use crate::models::{Bucket, GroupChange, MeasureGroup};

/// Upsert one group and its measures. A value that changed since last time has its old
/// form copied to `measure_revision` first; a group that had been marked deleted but is
/// present again has `deleted_at` cleared. Returns whether anything changed.
///
/// Runs on a connection the caller owns, so the group, its measures and any revision all
/// land in one transaction.
pub async fn upsert_group(conn: &mut PgConnection, g: &MeasureGroup) -> sqlx::Result<GroupChange> {
    let existed: bool =
        sqlx::query_scalar("SELECT exists(SELECT 1 FROM measure_group WHERE grpid = $1)")
            .bind(g.grpid)
            .fetch_one(&mut *conn)
            .await?;

    // The current values, to decide what counts as a revision.
    let mut current: HashMap<i32, (i64, i32)> = HashMap::new();
    if existed {
        let rows = sqlx::query("SELECT type_code, value, unit FROM measure WHERE grpid = $1")
            .bind(g.grpid)
            .fetch_all(&mut *conn)
            .await?;
        for r in rows {
            current.insert(r.get("type_code"), (r.get("value"), r.get("unit")));
        }
    }

    sqlx::query(
        "INSERT INTO measure_group \
           (grpid, person_id, ts, device_id, model, category, attrib, modified_at, deleted_at, raw, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, NULL, $9, now()) \
         ON CONFLICT (grpid) DO UPDATE SET person_id = EXCLUDED.person_id, ts = EXCLUDED.ts, \
           device_id = EXCLUDED.device_id, model = EXCLUDED.model, category = EXCLUDED.category, \
           attrib = EXCLUDED.attrib, modified_at = EXCLUDED.modified_at, deleted_at = NULL, \
           raw = EXCLUDED.raw, updated_at = now()",
    )
    .bind(g.grpid)
    .bind(g.person_id)
    .bind(g.ts)
    .bind(&g.device_id)
    .bind(g.model)
    .bind(g.category)
    .bind(g.attrib)
    .bind(g.modified_at)
    .bind(&g.raw)
    .execute(&mut *conn)
    .await?;

    let mut changed = false;
    for m in &g.measures {
        if let Some((old_value, old_unit)) = current.get(&m.type_code)
            && (*old_value != m.value || *old_unit != m.unit)
        {
            sqlx::query(
                "INSERT INTO measure_revision (grpid, type_code, value, unit) \
                 VALUES ($1, $2, $3, $4)",
            )
            .bind(g.grpid)
            .bind(m.type_code)
            .bind(old_value)
            .bind(old_unit)
            .execute(&mut *conn)
            .await?;
            changed = true;
        }
        sqlx::query(
            "INSERT INTO measure (grpid, type_code, value, unit, algo, fm) \
             VALUES ($1, $2, $3, $4, $5, $6) \
             ON CONFLICT (grpid, type_code) DO UPDATE SET value = EXCLUDED.value, \
               unit = EXCLUDED.unit, algo = EXCLUDED.algo, fm = EXCLUDED.fm",
        )
        .bind(g.grpid)
        .bind(m.type_code)
        .bind(m.value)
        .bind(m.unit)
        .bind(m.algo)
        .bind(m.fm)
        .execute(&mut *conn)
        .await?;
    }

    Ok(if !existed {
        GroupChange::New
    } else if changed {
        GroupChange::Revised
    } else {
        GroupChange::Unchanged
    })
}

/// Mark as deleted every non-deleted group whose `ts` is in `[from, to)` and whose grpid
/// is not in `present`. Returns how many were newly marked. A poll passes the window it
/// just read and the ids it saw, so a weigh-in the app no longer returns is flagged.
pub async fn mark_vanished(
    conn: &mut PgConnection,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    present: &[i64],
) -> sqlx::Result<u64> {
    let result = sqlx::query(
        "UPDATE measure_group SET deleted_at = now(), updated_at = now() \
         WHERE deleted_at IS NULL AND ts >= $1 AND ts < $2 AND NOT (grpid = ANY($3))",
    )
    .bind(from)
    .bind(to)
    .bind(present)
    .execute(conn)
    .await?;
    Ok(result.rows_affected())
}

/// One measured value for a tool answer.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct MeasureRow {
    pub grpid: i64,
    pub person_id: Option<i64>,
    pub ts: DateTime<Utc>,
    pub type_code: i32,
    pub type_name: Option<String>,
    pub value: f64,
    pub deleted: bool,
}

/// Measured values for a person over a window. `person = None` means the owner. `types`
/// empty means every type. Newest first, capped at `limit` groups.
pub async fn series(
    pool: &PgPool,
    person: Option<i64>,
    types: &[i32],
    after: Option<DateTime<Utc>>,
    before: Option<DateTime<Utc>>,
    include_deleted: bool,
    limit: i64,
) -> sqlx::Result<Vec<MeasureRow>> {
    // The groups to show, newest first, then every measure in them. All filtering is by
    // bound parameter; nothing is interpolated.
    sqlx::query_as(
        "WITH chosen AS ( \
            SELECT g.grpid FROM measure_group g \
            WHERE g.person_id = COALESCE($1, (SELECT id FROM person WHERE is_owner ORDER BY id LIMIT 1)) \
              AND ($2::timestamptz IS NULL OR g.ts >= $2) \
              AND ($3::timestamptz IS NULL OR g.ts <= $3) \
              AND ($4 OR g.deleted_at IS NULL) \
            ORDER BY g.ts DESC LIMIT $5 \
         ) \
         SELECT v.grpid, v.person_id, v.ts, v.type_code, v.type_name, v.value::double precision AS value, v.deleted \
         FROM measure_value v JOIN chosen c ON c.grpid = v.grpid \
         WHERE (array_length($6::int[], 1) IS NULL OR v.type_code = ANY($6)) \
         ORDER BY v.ts DESC, v.type_code",
    )
    .bind(person)
    .bind(after)
    .bind(before)
    .bind(include_deleted)
    .bind(limit)
    .bind(types)
    .fetch_all(pool)
    .await
}

/// One trend bucket.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct TrendRow {
    pub bucket: DateTime<Utc>,
    pub min: f64,
    pub avg: f64,
    pub max: f64,
    pub count: i64,
}

/// A single type aggregated per day/week/month in `tz`. Deleted groups are excluded.
pub async fn trend(
    pool: &PgPool,
    person: Option<i64>,
    type_code: i32,
    after: Option<DateTime<Utc>>,
    before: Option<DateTime<Utc>>,
    bucket: Bucket,
    tz: &str,
) -> sqlx::Result<Vec<TrendRow>> {
    // `bucket.trunc()` and `tz` are not caller input: the field name is one of three
    // constants and the timezone is the server's own setting.
    let sql = format!(
        "SELECT date_trunc('{trunc}', v.ts AT TIME ZONE $1) AT TIME ZONE $1 AS bucket, \
            min(v.value)::double precision AS min, avg(v.value)::double precision AS avg, max(v.value)::double precision AS max, count(*) AS count \
         FROM measure_value v \
         WHERE v.person_id = COALESCE($2, (SELECT id FROM person WHERE is_owner ORDER BY id LIMIT 1)) \
           AND v.type_code = $3 AND NOT v.deleted \
           AND ($4::timestamptz IS NULL OR v.ts >= $4) \
           AND ($5::timestamptz IS NULL OR v.ts <= $5) \
         GROUP BY bucket ORDER BY bucket",
        trunc = bucket.trunc()
    );
    sqlx::query_as(AssertSqlSafe(sql))
        .bind(tz)
        .bind(person)
        .bind(type_code)
        .bind(after)
        .bind(before)
        .fetch_all(pool)
        .await
}

/// The grpid of a person's newest non-deleted weigh-in.
pub async fn latest_group(pool: &PgPool, person: Option<i64>) -> sqlx::Result<Option<i64>> {
    sqlx::query_scalar(
        "SELECT grpid FROM measure_group \
         WHERE person_id = COALESCE($1, (SELECT id FROM person WHERE is_owner ORDER BY id LIMIT 1)) \
           AND deleted_at IS NULL ORDER BY ts DESC LIMIT 1",
    )
    .bind(person)
    .fetch_optional(pool)
    .await
}

/// Mark one group deleted (the getmeas `deleted` flag). Idempotent.
pub async fn mark_deleted(conn: &mut PgConnection, grpid: i64) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE measure_group SET deleted_at = COALESCE(deleted_at, now()), updated_at = now() \
         WHERE grpid = $1",
    )
    .bind(grpid)
    .execute(conn)
    .await?;
    Ok(())
}

/// Resolve a mix of numeric codes and type names to codes. An unrecognised name is
/// dropped (the caller gets no rows for it rather than an error).
pub async fn resolve_types(pool: &PgPool, wanted: &[String]) -> sqlx::Result<Vec<i32>> {
    let mut codes = Vec::new();
    let mut names = Vec::new();
    for w in wanted {
        match w.trim().parse::<i32>() {
            Ok(code) => codes.push(code),
            Err(_) => names.push(w.trim().to_lowercase()),
        }
    }
    if !names.is_empty() {
        let found: Vec<i32> =
            sqlx::query_scalar("SELECT code FROM measure_type WHERE lower(name) = ANY($1)")
                .bind(&names)
                .fetch_all(pool)
                .await?;
        codes.extend(found);
    }
    Ok(codes)
}
