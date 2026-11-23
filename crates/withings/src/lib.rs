//! The read-only Withings private-API client. Fetches and parses; no database.

pub mod api;
pub mod error;
pub mod session;

pub use api::{ALLOWED, Client, MeasPage, ScorePage};
pub use error::ClientError;
pub use session::Session;
