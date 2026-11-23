//! The server wiring that needs a database: the migration guard, `/healthz` and the
//! `Host` allowlist.
//!
//! The tool surface itself — names, descriptions, read-only annotations — is checked in
//! `withings-mcp-server`'s own unit tests, where no pool is needed to build the router.
//! `tests/test_mcp_server.py` drove the ASGI app through httpx for the same ends.

use sqlx::postgres::PgPool;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use withings_core::config::McpSettings;
use withings_core::db::DbError;
use withings_mcp_server::server;

use crate::support::{self, db_test};

/// A server on a free port, with the given `Host` allowlist.
async fn serve_with(
    allowed_hosts: &[&str],
) -> Option<(u16, tokio::task::JoinHandle<()>, McpSettings)> {
    let dsn = support::container_dsn().await?;
    let mut settings = McpSettings::for_dsn(dsn);
    settings.host = "127.0.0.1".into();
    settings.allowed_hosts = allowed_hosts.iter().map(|h| (*h).to_owned()).collect();
    // Port 0 asks the OS for a free one, so two runs never collide.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    settings.port = listener.local_addr().unwrap().port();

    let pool: PgPool = server::open_pool(&settings)
        .await
        .expect("the shared container is migrated");
    let allowed = settings.allowed_hosts.clone();
    let task = tokio::spawn(async move {
        axum::serve(
            listener,
            server::build_router(pool, allowed, "UTC".to_owned()),
        )
        .await
        .unwrap();
    });
    Some((settings.port, task, settings))
}

/// One HTTP/1.1 request with a chosen `Host`, returning the whole response.
///
/// Raw TCP because the point is to control the `Host` header exactly, and because an
/// HTTP client crate for a handful of requests is a dependency to keep patched for
/// nothing.
async fn request(port: u16, method: &str, path: &str, host: &str, body: &str) -> String {
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("the test server is listening");
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host}\r\n\
         Content-Type: application/json\r\n\
         Accept: application/json, text/event-stream\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    String::from_utf8_lossy(&response).into_owned()
}

async fn status_line(port: u16, method: &str, path: &str, host: &str, body: &str) -> String {
    request(port, method, path, host, body)
        .await
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .to_owned()
}

const TOOLS_LIST: &str = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#;

db_test!(startup_refuses_an_unmigrated_database, |f| {
    // The MCP role has no DDL rights, so a missing schema is fatal at startup, never
    // repaired. Failing here names the problem; failing per query blames the caller.
    let Some(dsn) = support::container_dsn().await else {
        return;
    };
    sqlx::query("DROP DATABASE IF EXISTS withings_mcp_unmigrated")
        .execute(&f.pool)
        .await
        .unwrap();
    sqlx::query("CREATE DATABASE withings_mcp_unmigrated")
        .execute(&f.pool)
        .await
        .unwrap();

    let (base, _) = dsn.rsplit_once('/').expect("the dsn names a database");
    let error = server::open_pool(&McpSettings::for_dsn(format!(
        "{base}/withings_mcp_unmigrated"
    )))
    .await
    .expect_err("an unmigrated database must be refused");
    assert!(
        matches!(&error, DbError::MigrationsMissing(missing) if missing.contains("0001_init.sql")),
        "{error}"
    );

    sqlx::query("DROP DATABASE IF EXISTS withings_mcp_unmigrated")
        .execute(&f.pool)
        .await
        .unwrap();
});

db_test!(healthz_answers_once_the_pool_is_open, |_f| {
    let Some((port, task, settings)) = serve_with(&["localhost", "127.0.0.1"]).await else {
        return;
    };
    let response = request(port, "GET", "/healthz", &format!("127.0.0.1:{port}"), "").await;
    assert!(
        response.lines().next().unwrap().contains("200"),
        "{response}"
    );
    assert!(response.ends_with("ok"), "{response}");
    // The `--health-check` subcommand, end to end against a running server. A Rust image
    // has neither Python nor a shell, so this is what replaces the healthcheck
    // `deploy/compose.yaml` currently runs.
    server::health_check(&settings.host, port)
        .await
        .expect("/healthz answers 200");
    task.abort();
});

db_test!(the_whole_tool_surface_is_reachable_over_http, |_f| {
    let Some((port, task, _)) = serve_with(&["localhost", "127.0.0.1"]).await else {
        return;
    };
    let response = request(
        port,
        "POST",
        "/mcp",
        &format!("127.0.0.1:{port}"),
        TOOLS_LIST,
    )
    .await;
    assert!(
        response.lines().next().unwrap().contains("200"),
        "{response}"
    );
    let names = ["status", "people", "measurements", "trend", "ecg", "sql"];
    for name in names {
        assert!(
            response.contains(&format!("\"name\":\"{name}\"")),
            "{name} is missing from tools/list"
        );
    }
    // Six tools, all read-only: no write tool slipped in.
    let count = response.matches("\"name\":\"").count();
    assert_eq!(
        count,
        names.len(),
        "expected six tools, saw {count}: {response}"
    );
    assert!(
        !response.contains("\"readOnlyHint\":false"),
        "a non-read-only tool is advertised: {response}"
    );
    task.abort();
});

db_test!(
    the_mcp_endpoint_answers_the_host_mcphub_addresses_it_by,
    |_f| {
        // MCPHub's registry row is `http://withings-mcp:8000/mcp`, so the inbound Host is
        // `withings-mcp:8000`. The allowlist entry carries no port on purpose, and rmcp
        // reads a portless entry as "this hostname on any port".
        let Some((port, task, _)) = serve_with(&["withings-mcp"]).await else {
            return;
        };
        for host in [
            "withings-mcp:8000",
            "withings-mcp",
            "withings-mcp:9999",
            "WITHINGS-MCP:8000",
        ] {
            let status = status_line(port, "POST", "/mcp", host, TOOLS_LIST).await;
            assert!(
                status.contains("200"),
                "Host {host:?} should have been accepted, got {status:?}"
            );
        }
        task.abort();
    }
);

db_test!(the_mcp_endpoint_rejects_a_host_it_was_not_given, |_f| {
    // The whole point of narrowing the list: a DNS-rebinding attempt arrives with
    // somebody else's Host.
    let Some((port, task, _)) = serve_with(&["withings-mcp"]).await else {
        return;
    };
    for host in ["evil.example.com", "localhost:8000", "10.0.0.5:8000"] {
        let status = status_line(port, "POST", "/mcp", host, TOOLS_LIST).await;
        assert!(
            status.contains("403"),
            "Host {host:?} should have been refused, got {status:?}"
        );
    }
    task.abort();
});

db_test!(healthz_answers_a_host_the_mcp_endpoint_rejects, |_f| {
    // This is what lets production set `WITHINGS_MCP_ALLOWED_HOSTS: withings-mcp`
    // alone, with no loopback entry: `/healthz` is a plain route on the root router and
    // only `/mcp` is nested onto the MCP service, so the guard never sees the probe. If
    // that ever changes, `withings-mcp --health-check` starts failing and nothing about
    // the container would explain why.
    let Some((port, task, settings)) = serve_with(&["withings-mcp"]).await else {
        return;
    };
    let probe_host = format!("127.0.0.1:{port}");
    assert!(
        status_line(port, "POST", "/mcp", &probe_host, TOOLS_LIST)
            .await
            .contains("403"),
        "the guard should refuse this Host on /mcp"
    );
    assert!(
        status_line(port, "GET", "/healthz", &probe_host, "")
            .await
            .contains("200"),
        "/healthz must answer the same Host regardless"
    );
    // And the subcommand the healthcheck actually runs agrees.
    server::health_check(&settings.host, port)
        .await
        .expect("--health-check must pass with only `withings-mcp` allowed");
    task.abort();
});

db_test!(the_health_check_fails_when_nothing_is_listening, |_f| {
    // Port 0 cannot be connected to, so this exercises the failure path rather than
    // racing a real server.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let error = server::health_check("127.0.0.1", port)
        .await
        .expect_err("nothing is listening");
    assert!(error.contains("could not connect"), "{error}");
});
