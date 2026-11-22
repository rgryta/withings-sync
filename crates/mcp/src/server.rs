//! The MCP server. Filled by the MCP task.
use withings_core::config::McpSettings;

pub const SERVER_NAME: &str = "withings";

/// Placeholder until the MCP task lands the real router.
pub async fn run(_settings: &McpSettings) -> Result<(), Box<dyn std::error::Error>> {
    Err("withings-mcp server not yet implemented".into())
}

pub async fn health_check(_host: &str, _port: u16) -> Result<(), String> {
    Err("not yet implemented".to_owned())
}
