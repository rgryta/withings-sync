//! Domain models: the shapes the store reads and writes. The ingester builds these from
//! Withings payloads; the MCP reads row structs the store returns.

use chrono::{DateTime, Utc};
use serde_json::Value;

/// A profile on the account. `id` is the Withings userid.
#[derive(Debug, Clone, PartialEq)]
pub struct Person {
    pub id: i64,
    pub label: Option<String>,
    pub is_owner: bool,
    pub shared: bool,
    pub raw: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Device {
    pub id: String,
    pub model: Option<String>,
    pub raw: Value,
}

/// One measured value inside a group. Real value = `value * 10^unit`.
#[derive(Debug, Clone, PartialEq)]
pub struct Measure {
    pub type_code: i32,
    pub value: i64,
    pub unit: i32,
    pub algo: Option<i32>,
    pub fm: Option<i32>,
}

/// One weigh-in / measurement session.
#[derive(Debug, Clone, PartialEq)]
pub struct MeasureGroup {
    pub grpid: i64,
    pub person_id: Option<i64>,
    pub ts: DateTime<Utc>,
    pub device_id: Option<String>,
    pub model: Option<i32>,
    pub category: Option<i16>,
    pub attrib: Option<i16>,
    pub modified_at: Option<DateTime<Utc>>,
    pub measures: Vec<Measure>,
    pub raw: Value,
}

/// The outcome of upserting a group: whether it was new, had a value change (old values
/// copied to `measure_revision`), or was already present unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupChange {
    New,
    Revised,
    Unchanged,
}

/// A derived score (healthindicator or vitality).
#[derive(Debug, Clone, PartialEq)]
pub struct IndicatorScore {
    pub id: i64,
    pub person_id: Option<i64>,
    pub source: String,
    pub score_type: Option<String>,
    pub ts: DateTime<Utc>,
    pub score: Option<f64>,
    pub status: Option<String>,
    pub raw: Value,
}

/// A weight/trend projection from `trajectory`.
#[derive(Debug, Clone, PartialEq)]
pub struct Trajectory {
    pub id: i64,
    pub person_id: Option<i64>,
    pub kind: Option<String>,
    pub active: Option<bool>,
    pub raw: Value,
}

/// A Body Scan ECG recording. Not written yet (no endpoint observed), kept for shape.
#[derive(Debug, Clone, PartialEq)]
pub struct Ecg {
    pub id: i64,
    pub person_id: Option<i64>,
    pub ts: DateTime<Utc>,
    pub device_id: Option<String>,
    pub heart_rate: Option<i32>,
    pub classification: Option<String>,
    pub sampling_hz: Option<i32>,
    pub signal: Option<Value>,
    pub raw: Value,
}

/// How a trend is bucketed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bucket {
    Day,
    Week,
    Month,
}

impl Bucket {
    /// The `date_trunc` field name.
    pub fn trunc(self) -> &'static str {
        match self {
            Bucket::Day => "day",
            Bucket::Week => "week",
            Bucket::Month => "month",
        }
    }
}
