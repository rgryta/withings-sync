//! ECG recordings. No endpoint has been observed on the owner's account, so nothing
//! writes these yet; the shape is here for when a recording first appears.

use chrono::{DateTime, Utc};
use sqlx::{PgConnection, postgres::PgPool};

use crate::models::Ecg;

/// Upsert a recording. Returns true when it is new.
pub async fn upsert(conn: &mut PgConnection, e: &Ecg) -> sqlx::Result<bool> {
    let existed: bool = sqlx::query_scalar("SELECT exists(SELECT 1 FROM ecg WHERE id = $1)")
        .bind(e.id)
        .fetch_one(&mut *conn)
        .await?;
    sqlx::query(
        "INSERT INTO ecg (id, person_id, ts, device_id, heart_rate, classification, \
            sampling_hz, signal, deleted_at, raw, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, NULL, $9, now()) \
         ON CONFLICT (id) DO UPDATE SET person_id = EXCLUDED.person_id, ts = EXCLUDED.ts, \
           device_id = EXCLUDED.device_id, heart_rate = EXCLUDED.heart_rate, \
           classification = EXCLUDED.classification, sampling_hz = EXCLUDED.sampling_hz, \
           signal = EXCLUDED.signal, deleted_at = NULL, raw = EXCLUDED.raw, updated_at = now()",
    )
    .bind(e.id)
    .bind(e.person_id)
    .bind(e.ts)
    .bind(&e.device_id)
    .bind(e.heart_rate)
    .bind(&e.classification)
    .bind(e.sampling_hz)
    .bind(&e.signal)
    .bind(&e.raw)
    .execute(&mut *conn)
    .await?;
    Ok(!existed)
}

#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct EcgRow {
    pub id: i64,
    pub person_id: Option<i64>,
    pub ts: DateTime<Utc>,
    pub heart_rate: Option<i32>,
    pub classification: Option<String>,
    pub sampling_hz: Option<i32>,
}

pub async fn list(pool: &PgPool, person: Option<i64>, limit: i64) -> sqlx::Result<Vec<EcgRow>> {
    sqlx::query_as(
        "SELECT id, person_id, ts, heart_rate, classification, sampling_hz FROM ecg \
         WHERE deleted_at IS NULL AND ($1::bigint IS NULL OR person_id = $1) \
         ORDER BY ts DESC LIMIT $2",
    )
    .bind(person)
    .bind(limit)
    .fetch_all(pool)
    .await
}
