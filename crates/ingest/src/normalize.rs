//! Turn Withings payloads into archive rows. Pure functions over `serde_json::Value`, so
//! the parsing is testable without a network or a database.
//!
//! A group carries no userid (getmeas is called per profile), so `person_id` is left
//! `None` here and the runner sets it to the profile it queried.

use chrono::{DateTime, TimeZone, Utc};
use serde_json::Value;
use withings_core::models::{Device, Ecg, Measure, MeasureGroup, Person};

#[derive(Debug, thiserror::Error, PartialEq)]
#[error("payload is missing or malformed: {0}")]
pub struct ShapeError(pub String);

fn require_i64(v: &Value, key: &str) -> Result<i64, ShapeError> {
    v.get(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| ShapeError(format!("missing i64 field {key}")))
}

fn epoch(v: &Value, key: &str) -> Option<DateTime<Utc>> {
    v.get(key)
        .and_then(Value::as_i64)
        .and_then(|s| Utc.timestamp_opt(s, 0).single())
}

/// A profile from `getuserslist`. `is_owner` is left false — the runner decides which
/// profile is the account holder's. `shared` comes from the listing.
pub fn person(v: &Value) -> Result<Person, ShapeError> {
    let id = require_i64(v, "id")?;
    let label = v
        .get("shortname")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .or_else(|| v.get("firstname").and_then(Value::as_str))
        .map(str::to_owned);
    let shared = v.get("isshared").and_then(Value::as_i64).unwrap_or(0) != 0;
    Ok(Person {
        id,
        label,
        is_owner: false,
        shared,
        raw: v.clone(),
    })
}

/// A device from `association`. The id is Withings' device string.
pub fn device(v: &Value) -> Result<Device, ShapeError> {
    let id = v
        .get("deviceid")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| {
            v.get("deviceid")
                .and_then(Value::as_i64)
                .map(|n| n.to_string())
        })
        .ok_or_else(|| ShapeError("device has no deviceid".to_owned()))?;
    let model = v
        .get("model")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| {
            v.get("model")
                .and_then(Value::as_i64)
                .map(|n| n.to_string())
        });
    Ok(Device {
        id,
        model,
        raw: v.clone(),
    })
}

/// One measurement group from `getmeas`. `person_id` is `None`; the runner assigns it.
pub fn group(v: &Value) -> Result<MeasureGroup, ShapeError> {
    let grpid = require_i64(v, "grpid")?;
    let ts = epoch(v, "date").ok_or_else(|| ShapeError(format!("group {grpid} has no date")))?;
    let device_id = v.get("deviceid").and_then(Value::as_str).map(str::to_owned);
    let measures = v
        .get("measures")
        .and_then(Value::as_array)
        .ok_or_else(|| ShapeError(format!("group {grpid} has no measures array")))?
        .iter()
        .map(|m| {
            Ok(Measure {
                type_code: require_i64(m, "type")? as i32,
                value: require_i64(m, "value")?,
                unit: require_i64(m, "unit")? as i32,
                algo: m.get("algo").and_then(Value::as_i64).map(|n| n as i32),
                fm: m.get("fm").and_then(Value::as_i64).map(|n| n as i32),
            })
        })
        .collect::<Result<Vec<_>, ShapeError>>()?;
    Ok(MeasureGroup {
        grpid,
        person_id: None,
        ts,
        device_id,
        model: v.get("model").and_then(Value::as_i64).map(|n| n as i32),
        category: v.get("category").and_then(Value::as_i64).map(|n| n as i16),
        attrib: v.get("attrib").and_then(Value::as_i64).map(|n| n as i16),
        modified_at: epoch(v, "modified"),
        measures,
        raw: v.clone(),
    })
}

/// Whether a getmeas group is flagged deleted upstream.
pub fn group_is_deleted(v: &Value) -> bool {
    v.get("deleted")
        .map(|d| {
            d.as_bool()
                .unwrap_or_else(|| d.as_i64().map(|n| n != 0).unwrap_or(false))
        })
        .unwrap_or(false)
}

/// An ECG recording. No endpoint is observed on the owner's account, so this is unused
/// today; kept so a first recording needs no new parser.
pub fn ecg(v: &Value, signal: Option<&Value>) -> Result<Ecg, ShapeError> {
    let id = require_i64(v, "signalid").or_else(|_| require_i64(v, "id"))?;
    let ts = epoch(v, "timestamp")
        .or_else(|| epoch(v, "date"))
        .ok_or_else(|| ShapeError(format!("ecg {id} has no timestamp")))?;
    Ok(Ecg {
        id,
        person_id: None,
        ts,
        device_id: v.get("deviceid").and_then(Value::as_str).map(str::to_owned),
        heart_rate: v
            .get("heart_rate")
            .and_then(Value::as_i64)
            .map(|n| n as i32),
        classification: v.get("afib").and_then(Value::as_str).map(str::to_owned),
        sampling_hz: v
            .get("sampling_frequency")
            .and_then(Value::as_i64)
            .map(|n| n as i32),
        signal: signal.cloned(),
        raw: v.clone(),
    })
}
