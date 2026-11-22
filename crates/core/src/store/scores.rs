//! Derived scores (healthindicator, vitality) and trajectories, as typed projections
//! over what the raw store already keeps.

use chrono::{DateTime, Utc};
use sqlx::{PgConnection, postgres::PgPool};

use crate::models::{IndicatorScore, Trajectory};

pub async fn upsert_score(conn: &mut PgConnection, s: &IndicatorScore) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO indicator_score (id, person_id, source, score_type, ts, score, status, deleted_at, raw, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, NULL, $8, now()) \
         ON CONFLICT (id) DO UPDATE SET person_id = EXCLUDED.person_id, source = EXCLUDED.source, \
           score_type = EXCLUDED.score_type, ts = EXCLUDED.ts, score = EXCLUDED.score, \
           status = EXCLUDED.status, deleted_at = NULL, raw = EXCLUDED.raw, updated_at = now()",
    )
    .bind(s.id)
    .bind(s.person_id)
    .bind(&s.source)
    .bind(&s.score_type)
    .bind(s.ts)
    .bind(s.score)
    .bind(&s.status)
    .bind(&s.raw)
    .execute(conn)
    .await?;
    Ok(())
}

pub async fn upsert_trajectory(conn: &mut PgConnection, t: &Trajectory) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO trajectory (id, person_id, type, active, deleted_at, raw, updated_at) \
         VALUES ($1, $2, $3, $4, NULL, $5, now()) \
         ON CONFLICT (id) DO UPDATE SET person_id = EXCLUDED.person_id, type = EXCLUDED.type, \
           active = EXCLUDED.active, deleted_at = NULL, raw = EXCLUDED.raw, updated_at = now()",
    )
    .bind(t.id)
    .bind(t.person_id)
    .bind(&t.kind)
    .bind(t.active)
    .bind(&t.raw)
    .execute(conn)
    .await?;
    Ok(())
}

/// A score for a tool answer.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct ScoreRow {
    pub source: String,
    pub score_type: Option<String>,
    pub ts: DateTime<Utc>,
    pub score: Option<f64>,
    pub status: Option<String>,
}

pub async fn latest(pool: &PgPool, person: Option<i64>) -> sqlx::Result<Vec<ScoreRow>> {
    // The newest score of each (source, score_type) for the person.
    sqlx::query_as(
        "SELECT DISTINCT ON (source, score_type) source, score_type, ts, score, status \
         FROM indicator_score \
         WHERE person_id = COALESCE($1, (SELECT id FROM person WHERE is_owner ORDER BY id LIMIT 1)) \
           AND deleted_at IS NULL \
         ORDER BY source, score_type, ts DESC",
    )
    .bind(person)
    .fetch_all(pool)
    .await
}
