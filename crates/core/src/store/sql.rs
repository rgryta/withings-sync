//! The read-only SQL escape hatch behind the `sql` tool.
//!
//! Three guards, not one: the transaction is `READ ONLY` so a write errors, the MCP role
//! holds no write grant and cannot see `ingest_session`, and `statement_timeout` cancels
//! a query that runs too long. The query is wrapped as a subquery yielding one JSON row
//! each, so any column type comes back without per-type decoding here.

use std::time::Duration;

use serde_json::Value;
use sqlx::AssertSqlSafe;
use sqlx::postgres::PgPool;

#[derive(Debug, Clone, PartialEq)]
pub struct SqlResult {
    pub rows: Vec<Value>,
    pub truncated: bool,
}

/// Run a caller's SELECT read-only, capped and time-bounded. Errors (a write, a denied
/// table, a timeout) come back as the `sqlx::Error` they are, for the tool to render.
pub async fn run_readonly(
    pool: &PgPool,
    query: &str,
    row_cap: i64,
    timeout: Duration,
) -> sqlx::Result<SqlResult> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION READ ONLY")
        .execute(&mut *tx)
        .await?;
    // Not caller input: the timeout is the server's own setting, formatted as an integer.
    let ms = timeout.as_millis().min(i64::MAX as u128) as i64;
    sqlx::query(AssertSqlSafe(format!("SET LOCAL statement_timeout = {ms}")))
        .execute(&mut *tx)
        .await?;
    // One more row than the cap, so the tool can say the result was truncated.
    let trimmed = query.trim().trim_end_matches(';');
    let wrapped = format!(
        "SELECT to_jsonb(t) FROM ({trimmed}) t LIMIT {}",
        row_cap + 1
    );
    let rows: Vec<Value> = sqlx::query_scalar(AssertSqlSafe(wrapped))
        .fetch_all(&mut *tx)
        .await?;
    tx.rollback().await?;
    let truncated = rows.len() as i64 > row_cap;
    Ok(SqlResult {
        rows: rows.into_iter().take(row_cap as usize).collect(),
        truncated,
    })
}
