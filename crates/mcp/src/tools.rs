//! The tool layer: it resolves arguments, calls the store, and hands rows to the
//! renderer. It holds no SQL of its own — every query lives in `withings_core::store`.

use std::time::Duration;

use chrono::{DateTime, Utc};
use sqlx::postgres::PgPool;
use withings_core::models::Bucket;
use withings_core::store::{ecg, measures, people, sql, sync};

use crate::render;

#[derive(Debug)]
pub enum ToolError {
    /// The caller's mistake: bad argument, surfaced as `invalid_params`.
    Usage(String),
    /// Ours: a failed read, logged in full and reported blandly.
    Sql(sqlx::Error),
}

impl From<sqlx::Error> for ToolError {
    fn from(e: sqlx::Error) -> Self {
        ToolError::Sql(e)
    }
}

type R = Result<String, ToolError>;

/// Parse an ISO 8601 stamp or a Unix epoch.
fn parse_when(value: Option<&str>) -> Result<Option<DateTime<Utc>>, ToolError> {
    let Some(raw) = value.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    if let Ok(dt) = DateTime::parse_from_rfc3339(raw) {
        return Ok(Some(dt.with_timezone(&Utc)));
    }
    if let Ok(epoch) = raw.parse::<i64>()
        && let Some(dt) = DateTime::from_timestamp(epoch, 0)
    {
        return Ok(Some(dt));
    }
    Err(ToolError::Usage(format!(
        "not a time I can read (ISO 8601 or a Unix epoch): {raw:?}"
    )))
}

fn clamp(limit: Option<i64>, default: i64, ceiling: i64) -> i64 {
    limit.unwrap_or(default).clamp(1, ceiling)
}

pub async fn status(pool: &PgPool) -> R {
    let loops = sync::status(pool).await?;
    let people = people::list(pool).await?;
    Ok(render::status(&loops, &people))
}

pub async fn people_tool(pool: &PgPool) -> R {
    Ok(render::people(&people::list(pool).await?))
}

#[allow(clippy::too_many_arguments)]
pub async fn measurements(
    pool: &PgPool,
    person: Option<i64>,
    after: Option<&str>,
    before: Option<&str>,
    types: &[String],
    include_deleted: bool,
    limit: Option<i64>,
) -> R {
    let after = parse_when(after)?;
    let before = parse_when(before)?;
    let codes = measures::resolve_types(pool, types).await?;
    let rows = measures::series(
        pool,
        person,
        &codes,
        after,
        before,
        include_deleted,
        clamp(limit, 20, 500),
    )
    .await?;
    Ok(render::measurements(&rows))
}

pub async fn trend(
    pool: &PgPool,
    type_: &str,
    person: Option<i64>,
    after: Option<&str>,
    before: Option<&str>,
    bucket: Option<&str>,
    tz: &str,
) -> R {
    let codes = measures::resolve_types(pool, std::slice::from_ref(&type_.to_owned())).await?;
    let code = *codes
        .first()
        .ok_or_else(|| ToolError::Usage(format!("not a measure type I know: {type_:?}")))?;
    let bucket = match bucket.unwrap_or("week") {
        "day" => Bucket::Day,
        "week" => Bucket::Week,
        "month" => Bucket::Month,
        other => {
            return Err(ToolError::Usage(format!(
                "bucket is day, week or month, not {other:?}"
            )));
        }
    };
    let after = parse_when(after)?;
    let before = parse_when(before)?;
    let rows = measures::trend(pool, person, code, after, before, bucket, tz).await?;
    Ok(render::trend(type_, bucket, &rows))
}

pub async fn ecg_tool(pool: &PgPool, person: Option<i64>, limit: Option<i64>) -> R {
    let rows = ecg::list(pool, person, clamp(limit, 20, 200)).await?;
    Ok(render::ecg(&rows))
}

pub async fn sql_tool(pool: &PgPool, query: &str) -> R {
    let q = query.trim();
    if q.is_empty() {
        return Err(ToolError::Usage("the query is empty".to_owned()));
    }
    match sql::run_readonly(pool, q, 500, Duration::from_secs(10)).await {
        Ok(result) => Ok(render::sql(&result)),
        // A write, a denied table or a timeout is the caller's to see, not a 500: the
        // message says which, and the read-only transaction means nothing changed.
        Err(e) => Err(ToolError::Usage(format!("query failed: {e}"))),
    }
}
