//! Loop bookkeeping: last runs, the auth-error flag, and the daily request budget.

use chrono::{DateTime, Utc};
use sqlx::{Row, postgres::PgPool};

pub async fn record_ok(pool: &PgPool, loop_: &str) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO sync_state (loop, last_run_at, last_ok_at, last_error) \
         VALUES ($1, now(), now(), NULL) \
         ON CONFLICT (loop) DO UPDATE SET last_run_at = now(), last_ok_at = now(), last_error = NULL",
    )
    .bind(loop_)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn record_error(pool: &PgPool, loop_: &str, error: &str) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO sync_state (loop, last_run_at, last_error) VALUES ($1, now(), $2) \
         ON CONFLICT (loop) DO UPDATE SET last_run_at = now(), last_error = EXCLUDED.last_error",
    )
    .bind(loop_)
    .bind(error)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn record_auth_error(pool: &PgPool, account: &str, error: &str) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO account (id, last_auth_error, last_auth_error_at, updated_at) \
         VALUES ($1, $2, now(), now()) \
         ON CONFLICT (id) DO UPDATE SET last_auth_error = EXCLUDED.last_auth_error, \
           last_auth_error_at = now(), updated_at = now()",
    )
    .bind(account)
    .bind(error)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn clear_auth_error(pool: &PgPool, account: &str) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE account SET last_auth_error = NULL, last_auth_error_at = NULL, updated_at = now() \
         WHERE id = $1 AND last_auth_error IS NOT NULL",
    )
    .bind(account)
    .execute(pool)
    .await?;
    Ok(())
}

/// Try to spend `n` requests against today's `ceiling`. The day is UTC, counted in the
/// row, so a restart does not hand out a fresh allowance. Returns false when the ceiling
/// is reached; the backfill then waits for the next day.
pub async fn take_budget(pool: &PgPool, loop_: &str, n: i64, ceiling: i64) -> sqlx::Result<bool> {
    let mut tx = pool.begin().await?;
    let row = sqlx::query(
        "INSERT INTO sync_state (loop, budget_day, budget_spent) VALUES ($1, current_date, 0) \
         ON CONFLICT (loop) DO UPDATE SET \
           budget_day = CASE WHEN sync_state.budget_day = current_date THEN sync_state.budget_day ELSE current_date END, \
           budget_spent = CASE WHEN sync_state.budget_day = current_date THEN sync_state.budget_spent ELSE 0 END \
         RETURNING budget_spent",
    )
    .bind(loop_)
    .fetch_one(&mut *tx)
    .await?;
    let spent: i64 = row.get("budget_spent");
    if spent + n > ceiling {
        tx.commit().await?;
        return Ok(false);
    }
    sqlx::query("UPDATE sync_state SET budget_spent = budget_spent + $2 WHERE loop = $1")
        .bind(loop_)
        .bind(n)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(true)
}

/// A loop's state for the `status` tool.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct StatusRow {
    pub loop_name: String,
    pub last_run_at: Option<DateTime<Utc>>,
    pub last_ok_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub budget_day: Option<chrono::NaiveDate>,
    pub budget_spent: i64,
    pub backfill_done: bool,
}

pub async fn status(pool: &PgPool) -> sqlx::Result<Vec<StatusRow>> {
    sqlx::query_as(
        "SELECT loop AS loop_name, last_run_at, last_ok_at, last_error, budget_day, \
            budget_spent, backfill_done FROM sync_state ORDER BY loop",
    )
    .fetch_all(pool)
    .await
}
