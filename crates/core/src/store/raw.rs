//! The verbatim response store: every payload the client fetches, kept whole.

use serde_json::Value;
use sqlx::PgConnection;

/// Keep a response verbatim under its endpoint kind and natural key. A changed body
/// overwrites; the typed projections keep the history that matters.
pub async fn upsert(
    conn: &mut PgConnection,
    kind: &str,
    key: &str,
    body: &Value,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO raw_response (kind, key, body, fetched_at) VALUES ($1, $2, $3, now()) \
         ON CONFLICT (kind, key) DO UPDATE SET body = EXCLUDED.body, fetched_at = now()",
    )
    .bind(kind)
    .bind(key)
    .bind(body)
    .execute(conn)
    .await?;
    Ok(())
}
