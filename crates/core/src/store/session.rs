//! The session jar: the persistent sessionid plus the rotating token, as JSON. The MCP
//! role cannot read this table (migration 0001 revokes it).

use serde_json::Value;
use sqlx::{PgConnection, postgres::PgPool};

/// The stored session for an account, or `None` before it is seeded.
pub async fn load(pool: &PgPool, account: &str) -> sqlx::Result<Option<Value>> {
    sqlx::query_scalar("SELECT session FROM ingest_session WHERE account_id = $1")
        .bind(account)
        .fetch_optional(pool)
        .await
}

/// Persist the session, replacing any prior one. Called in the same transaction as the
/// write that follows a token rotation, so a crash cannot drop the new token.
pub async fn save(conn: &mut PgConnection, account: &str, session: &Value) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO ingest_session (account_id, session, updated_at) VALUES ($1, $2, now()) \
         ON CONFLICT (account_id) DO UPDATE SET session = EXCLUDED.session, updated_at = now()",
    )
    .bind(account)
    .bind(session)
    .execute(conn)
    .await?;
    Ok(())
}
