//! The Withings ingester: poll and backfill, turning payloads into archive rows.

pub mod config;
pub mod normalize;
pub mod runner;
pub mod source;
