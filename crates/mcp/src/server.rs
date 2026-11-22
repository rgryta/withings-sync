//! The MCP server: streamable HTTP at `/mcp`, eight tools, no Withings credential.
//!
//! The pool is opened before the listener binds and the shipped migrations are asserted
//! present — never applied. This process runs as a role with no DDL rights, and a schema
//! it cannot create is a schema it must not pretend to have.
//!
//! Seven of the eight tools are reads. The eighth writes the two alias tables and
//! nothing else, which is a database fact here rather than a convention: the grant in
//! `0001_init.sql` is all the write authority `withings_mcp` has.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, ContentBlock, ErrorData, Implementation, ServerCapabilities, ServerConfig,
};
use rmcp::transport::streamable_http_server::session::never::NeverSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ServerHandler, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use sqlx::postgres::PgPool;
use withings_core::config::McpSettings;
use withings_core::db::{DbError, assert_migrations, create_pool};

use crate::tools::{self, ToolError};

/// What the server tells a client it is.
pub const INSTRUCTIONS: &str = "\
Withings body measurements, from a permanent archive in PostgreSQL written by a separate
ingester that reads the Withings app's own API. Nothing here reaches Withings live — only
what has already been archived, and nothing here can change anything on the account.

Values are stored in their real units (weight in kg, ratios in %, heart rate in bpm) and
timestamps are ISO 8601. Each weigh-in is a group of measured values; a value is named
when its type code is known, else shown as `type <code>`.

Every profile on the account is archived. `person` is a profile's userid, from `people`;
omit it and tools answer for the owner's own profile. A weigh-in the scale could not
attribute to anyone is kept with no person and is left out of a person's series.

`measurements` reads values by weigh-in, `trend` aggregates one type over time, and `sql`
runs a read-only query when neither fits. `status` says how much history is archived — an
empty answer can mean \"not synced yet\" rather than \"never measured\".
";

/// The name the server reports to clients, which MCPHub joins to each tool name with a
/// `-`: `withings-measurements`.
pub const SERVER_NAME: &str = "withings";

#[derive(Clone)]
pub struct WithingsMcp {
    pool: PgPool,
    tz: String,
}

/// Turn a tool's answer into a protocol response.
///
/// A usage error is the caller's and goes back as `invalid_params` with its own text. A
/// SQL error is ours: it is logged in full and the caller is told only that the read
/// failed, because a Postgres error string can carry the DSN and the data.
fn finish(result: Result<String, ToolError>) -> Result<CallToolResult, ErrorData> {
    match result {
        Ok(body) => Ok(CallToolResult::success(vec![ContentBlock::text(body)])),
        Err(ToolError::Usage(message)) => Err(ErrorData::invalid_params(message, None)),
        Err(ToolError::Sql(error)) => {
            tracing::error!(%error, "the archive could not be read");
            Err(ErrorData::internal_error(
                "the archive could not be read".to_string(),
                None,
            ))
        }
    }
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct NoArgs {}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct MeasurementsArgs {
    /// The profile's userid. Omit for the owner's own profile.
    pub person: Option<i64>,
    pub after: Option<String>,
    pub before: Option<String>,
    /// Measure types to include, by name (weight, fat_mass, ...) or numeric code. Empty
    /// means every type.
    #[serde(default)]
    pub types: Vec<String>,
    #[serde(default)]
    pub include_deleted: bool,
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TrendArgs {
    /// A measure type by name (weight, fat_ratio, ...) or numeric code.
    pub r#type: String,
    pub person: Option<i64>,
    pub after: Option<String>,
    pub before: Option<String>,
    /// day, week or month. Defaults to week.
    pub bucket: Option<String>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct EcgArgs {
    pub person: Option<i64>,
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SqlArgs {
    /// A read-only SELECT over the archive. Runs in a READ ONLY transaction with a
    /// statement timeout and a row cap; writes and the session table are refused.
    pub query: String,
}

// Tool names are hyphenated and declared explicitly: MCPHub joins them to the server
// name with `-`, so these read `withings-measurements`. Every tool is read-only and says
// so, so a client can tell without parsing the description.
#[tool_router]
impl WithingsMcp {
    pub fn new(pool: PgPool, tz: String) -> Self {
        Self { pool, tz }
    }

    #[tool(
        name = "status",
        description = "Ingest health: what each loop last did, whether the Withings session is \
                       still alive, how far backfill has got, and every profile with its weigh-in \
                       count. An empty measurement answer can mean \"not archived yet\" rather than \
                       \"never measured\" — check here first.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn status(&self, Parameters(_): Parameters<NoArgs>) -> Result<CallToolResult, ErrorData> {
        finish(tools::status(&self.pool).await)
    }

    #[tool(
        name = "people",
        description = "Every profile on the account, with its userid, whether it is the owner's or \
                       a shared profile, and how many weigh-ins it has. The userid is what the \
                       other tools take as `person`.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn people(&self, Parameters(_): Parameters<NoArgs>) -> Result<CallToolResult, ErrorData> {
        finish(tools::people_tool(&self.pool).await)
    }

    #[tool(
        name = "measurements",
        description = "Measured values grouped by weigh-in, newest first. `person` is a profile's \
                       userid and defaults to the owner. `types` filters by name (weight, fat_mass, \
                       muscle_mass, hydration, bone_mass, visceral_fat, heart_pulse, ...) or numeric \
                       code; omit for every value. `after`/`before` (ISO 8601 or a Unix epoch) \
                       narrow the range. Deleted weigh-ins are hidden unless `include_deleted`. \
                       Values are already in their real units (kg, %, bpm).",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn measurements(
        &self,
        Parameters(args): Parameters<MeasurementsArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        finish(
            tools::measurements(
                &self.pool,
                args.person,
                args.after.as_deref(),
                args.before.as_deref(),
                &args.types,
                args.include_deleted,
                args.limit,
            )
            .await,
        )
    }

    #[tool(
        name = "trend",
        description = "One measure type over time, aggregated per day, week or month (default \
                       week): average, min, max and count per bucket, bucketed in the archive's \
                       local timezone. `type` is a name or numeric code; `person` defaults to the \
                       owner. This is the tool for \"how has my weight changed\".",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn trend(
        &self,
        Parameters(args): Parameters<TrendArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        finish(
            tools::trend(
                &self.pool,
                &args.r#type,
                args.person,
                args.after.as_deref(),
                args.before.as_deref(),
                args.bucket.as_deref(),
                &self.tz,
            )
            .await,
        )
    }

    #[tool(
        name = "ecg",
        description = "ECG recordings, newest first, with heart rate and classification. The \
                       owner's account has none archived today (no device records them); the tool \
                       exists for when one appears.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn ecg(
        &self,
        Parameters(args): Parameters<EcgArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        finish(tools::ecg_tool(&self.pool, args.person, args.limit).await)
    }

    #[tool(
        name = "sql",
        description = "Run a read-only SELECT over the archive when the other tools do not fit. \
                       Tables: person, measure_group, measure, measure_value (view: scaled value \
                       plus type_name), measure_type, indicator_score, trajectory, sync_state. The \
                       query runs READ ONLY with a 10s timeout and a 500-row cap; writes and the \
                       session table are refused.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn sql(
        &self,
        Parameters(args): Parameters<SqlArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        finish(tools::sql_tool(&self.pool, &args.query).await)
    }
}

#[tool_handler]
impl ServerHandler for WithingsMcp {
    /// Written out rather than left to `#[tool_handler(instructions = "…")]`: that
    /// argument takes a literal, and a literal folded to fit this file would drop the
    /// hard wrap the Python server sends. This is what keeps the two byte-identical.
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(SERVER_NAME, env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS.to_string())
    }
}

/// Own the pool and refuse an unmigrated database.
///
/// Separate from [`run`] so a test can call it without a listener.
pub async fn open_pool(settings: &McpSettings) -> Result<PgPool, DbError> {
    let pool = create_pool(settings.dsn.expose()).await?;
    if let Err(error) = assert_migrations(&pool).await {
        // This process runs as a role with no DDL rights. A schema it cannot create is
        // one it must not pretend to have: failing at startup names the problem, where
        // serving on regardless would fail one query at a time and blame the caller.
        pool.close().await;
        return Err(error);
    }
    Ok(pool)
}

/// The container healthcheck's endpoint: a live query, not a liveness constant.
///
/// The pool is opened before the listener binds, so by the time this can answer at all
/// there is a pool — the Python's "starting" case is a refused connection here, which
/// Docker reads as unhealthy just the same. What is left worth checking is the database,
/// because `compose.yaml` has `autoheal=true` on this container and a server that cannot
/// reach Postgres must not report itself well.
async fn healthz(pool: PgPool) -> Response {
    match sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(&pool)
        .await
    {
        Ok(_) => "ok".into_response(),
        Err(error) => {
            tracing::error!(%error, "the healthcheck could not reach the database");
            (StatusCode::SERVICE_UNAVAILABLE, "database unreachable").into_response()
        }
    }
}

/// `/mcp` plus `/healthz`.
///
/// `/healthz` is a plain route on this router and only `/mcp` is nested onto the MCP
/// service, so the `Host` allowlist below does not apply to it and `--health-check`
/// needs no entry of its own.
/// `tests/archive/server.rs::healthz_answers_a_host_the_mcp_endpoint_rejects` holds
/// that: if it ever stops being true the healthcheck starts failing for a reason
/// nothing about the container would explain.
pub fn build_router(pool: PgPool, allowed_hosts: Vec<String>, tz: String) -> Router {
    let probe = pool.clone();
    let service = StreamableHttpService::new(
        move || Ok(WithingsMcp::new(pool.clone(), tz.clone())),
        Arc::new(NeverSessionManager::default()),
        stateless_config(allowed_hosts),
    );
    Router::new()
        .route("/healthz", get(move || healthz(probe.clone())))
        .nest_service("/mcp", service)
}

fn stateless_config(allowed_hosts: Vec<String>) -> StreamableHttpServerConfig {
    let mut config = StreamableHttpServerConfig::default();
    // There is no per-client state to keep, and statelessness takes session affinity off
    // the table behind MCPHub — what `stateless_http=True` bought the Python server.
    config.legacy_session_mode = false;
    // rmcp's guard against DNS rebinding. An empty list would mean "answer any `Host`",
    // so the list handed in here is never empty — see `DEFAULT_ALLOWED_HOSTS`. MCPHub
    // addresses this server by container name, so production sets `withings-mcp`.
    config.allowed_hosts = allowed_hosts;
    config
}

pub async fn run(settings: &McpSettings) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let pool = open_pool(settings).await?;
    let address: SocketAddr = format!("{}:{}", settings.host, settings.port).parse()?;
    let listener = tokio::net::TcpListener::bind(address).await?;
    // Logged because the failure mode is a server that boots clean and then refuses
    // every tool call with a 403: one line here turns a baffling outage into an obvious
    // one. `/healthz` is outside this list, so a container can look healthy while every
    // call is rejected.
    tracing::info!(
        allowed_hosts = %settings.allowed_hosts.join(", "),
        "answering /mcp only for these Host headers"
    );
    tracing::info!(%address, "serving the withings archive over MCP");
    axum::serve(
        listener,
        build_router(pool, settings.allowed_hosts.clone(), settings.tz.clone()),
    )
    .with_graceful_shutdown(async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await?;
    Ok(())
}

/// Probe our own `/healthz`, for the container healthcheck.
///
/// The image is `scratch`: no shell, no curl, no interpreter, so the healthcheck in
/// `deploy/compose.yaml` runs this binary with `--health-check` instead.
///
/// Hand-rolled over TCP rather than through an HTTP client crate: it reads one status
/// line from a loopback socket, and a dependency for that is a dependency to keep
/// patched for no gain. It is also the one thing in this binary that opens a socket at
/// all, which is why it speaks to loopback and nothing else.
pub async fn health_check(host: &str, port: u16) -> Result<(), String> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    // The bind address may be the wildcard, which is not a destination.
    let host = match host {
        "0.0.0.0" | "::" | "" => "127.0.0.1",
        host => host,
    };
    let target = format!("{host}:{port}");
    let mut stream = tokio::net::TcpStream::connect(&target)
        .await
        .map_err(|error| format!("could not connect to {target}: {error}"))?;
    stream
        .write_all(
            format!("GET /healthz HTTP/1.1\r\nHost: {target}\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .await
        .map_err(|error| format!("could not send the probe: {error}"))?;

    let mut status = String::new();
    BufReader::new(stream)
        .read_line(&mut status)
        .await
        .map_err(|error| format!("no answer from {target}: {error}"))?;
    if status.contains(" 200 ") {
        return Ok(());
    }
    Err(format!("/healthz answered {}", status.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool_names() -> Vec<String> {
        WithingsMcp::tool_router()
            .list_all()
            .into_iter()
            .map(|tool| tool.name.to_string())
            .collect()
    }

    #[test]
    fn every_tool_is_registered() {
        let mut names = tool_names();
        names.sort();
        assert_eq!(
            names,
            vec!["ecg", "measurements", "people", "sql", "status", "trend"]
        );
    }

    #[test]
    fn names_are_hyphenated_not_mixed() {
        // MCPHub joins server and tool with `-`; an underscore would read as
        // `withings-set_alias`, mixing both separators in one identifier.
        for name in tool_names() {
            assert!(!name.contains('_'), "{name}");
        }
    }

    #[test]
    fn tools_describe_themselves() {
        // MCPHub shows these descriptions; an unlabelled tool is an unusable one.
        for tool in WithingsMcp::tool_router().list_all() {
            assert!(tool.description.is_some(), "{}", tool.name);
        }
    }

    #[test]
    fn every_tool_only_reads() {
        // The whole archive is read-only toward Withings; no tool may advertise a write.
        for tool in WithingsMcp::tool_router().list_all() {
            let annotations = tool
                .annotations
                .as_ref()
                .unwrap_or_else(|| panic!("{} has no annotations", tool.name));
            assert_eq!(annotations.read_only_hint, Some(true), "{}", tool.name);
        }
    }

    #[test]
    fn the_instructions_say_what_the_archive_is_and_is_not() {
        assert!(INSTRUCTIONS.contains("body measurements"));
        assert!(
            INSTRUCTIONS.contains("cannot reach Withings live")
                || INSTRUCTIONS.contains("never reaches Withings")
                || INSTRUCTIONS.contains("Nothing here reaches Withings live")
        );
        assert!(INSTRUCTIONS.contains("Every profile on the account is archived"));
    }

    #[tokio::test]
    async fn the_health_check_target_is_never_the_wildcard_address() {
        // Connecting to 0.0.0.0 is not a thing the healthcheck can do; port 1 is closed,
        // so the error names the address it actually tried.
        let error = health_check("0.0.0.0", 1).await.unwrap_err();
        assert!(error.contains("127.0.0.1:1"), "{error}");
    }
}
