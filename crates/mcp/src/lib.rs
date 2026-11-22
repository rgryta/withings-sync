//! The read surface: MCP tools over the archive. No Withings credential, no HTTP client
//! out to Withings — only Postgres and the server's own callers.

pub mod render;
pub mod server;
pub mod tools;
