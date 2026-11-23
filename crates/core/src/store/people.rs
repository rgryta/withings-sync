//! Profiles and devices.

use sqlx::{PgConnection, postgres::PgPool};

use crate::models::{Device, Person};

/// Upsert a profile. `is_owner`/`shared` are set from what the listing says.
pub async fn upsert(conn: &mut PgConnection, p: &Person) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO person (id, label, is_owner, shared, raw, updated_at) \
         VALUES ($1, $2, $3, $4, $5, now()) \
         ON CONFLICT (id) DO UPDATE SET label = EXCLUDED.label, is_owner = EXCLUDED.is_owner, \
           shared = EXCLUDED.shared, raw = EXCLUDED.raw, updated_at = now()",
    )
    .bind(p.id)
    .bind(&p.label)
    .bind(p.is_owner)
    .bind(p.shared)
    .bind(&p.raw)
    .execute(conn)
    .await?;
    Ok(())
}

pub async fn upsert_device(conn: &mut PgConnection, d: &Device) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO device (id, model, raw, updated_at) VALUES ($1, $2, $3, now()) \
         ON CONFLICT (id) DO UPDATE SET model = EXCLUDED.model, raw = EXCLUDED.raw, updated_at = now()",
    )
    .bind(&d.id)
    .bind(&d.model)
    .bind(&d.raw)
    .execute(conn)
    .await?;
    Ok(())
}

/// A profile with its weigh-in span, for the `people` tool.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct PersonRow {
    pub id: i64,
    pub label: Option<String>,
    pub is_owner: bool,
    pub shared: bool,
    pub weigh_ins: i64,
    pub first_at: Option<chrono::DateTime<chrono::Utc>>,
    pub last_at: Option<chrono::DateTime<chrono::Utc>>,
}

pub async fn list(pool: &PgPool) -> sqlx::Result<Vec<PersonRow>> {
    sqlx::query_as(
        "SELECT p.id, p.label, p.is_owner, p.shared, \
            count(g.grpid) FILTER (WHERE g.deleted_at IS NULL) AS weigh_ins, \
            min(g.ts) FILTER (WHERE g.deleted_at IS NULL) AS first_at, \
            max(g.ts) FILTER (WHERE g.deleted_at IS NULL) AS last_at \
         FROM person p LEFT JOIN measure_group g ON g.person_id = p.id \
         GROUP BY p.id ORDER BY p.is_owner DESC, p.id",
    )
    .fetch_all(pool)
    .await
}

/// The owner's userid, when one profile is marked `is_owner`.
pub async fn owner_id(pool: &PgPool) -> sqlx::Result<Option<i64>> {
    sqlx::query_scalar("SELECT id FROM person WHERE is_owner ORDER BY id LIMIT 1")
        .fetch_optional(pool)
        .await
}

/// Ensure a person row exists for an id, without clobbering a label a sync already set.
/// Backfill learns the owner before a profile sync has run, and the measure FK needs the
/// row present.
pub async fn ensure(conn: &mut PgConnection, id: i64, is_owner: bool) -> sqlx::Result<()> {
    sqlx::query("INSERT INTO person (id, is_owner) VALUES ($1, $2) ON CONFLICT (id) DO NOTHING")
        .bind(id)
        .bind(is_owner)
        .execute(conn)
        .await?;
    Ok(())
}
