//! The credential-isolation guard.
//!
//! The MCP process holds no Withings credential and must not be able to reach Withings,
//! and in Cargo that is a link-time fact: the binary's package does not depend on
//! `withings-client`, so the code that could use a session is not in the process at all.
//! What is left to check is that the manifests still say so, that the settings type still
//! has nowhere to put a credential, and that the tools layer holds no SQL of its own.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn manifest(path: &str) -> String {
    std::fs::read_to_string(repo_root().join(path)).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn source(path: &str) -> String {
    std::fs::read_to_string(repo_root().join(path)).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn section(path: &str, name: &str) -> String {
    let manifest = manifest(path);
    let after = manifest
        .split_once(&format!("\n[{name}]"))
        .unwrap_or_else(|| panic!("{path} has no [{name}] section"))
        .1;
    after
        .split_once("\n[")
        .map_or(after, |(section, _)| section)
        .to_owned()
}

#[test]
fn the_mcp_binary_does_not_depend_on_the_withings_client() {
    let dependencies = section("Cargo.toml", "dependencies");
    assert!(
        !dependencies.contains("withings-client"),
        "the MCP binary must not link withings-client:\n{dependencies}"
    );
}

#[test]
fn the_mcp_binary_does_not_depend_on_the_ingester_either() {
    let dependencies = section("Cargo.toml", "dependencies");
    assert!(
        !dependencies.contains("withings-ingest"),
        "the MCP binary must not link withings-ingest:\n{dependencies}"
    );
}

#[test]
fn neither_the_core_nor_the_mcp_crate_depends_on_the_withings_client() {
    for path in ["crates/core/Cargo.toml", "crates/mcp/Cargo.toml"] {
        assert!(
            !manifest(path).contains("withings-client"),
            "{path} must not depend on withings-client"
        );
    }
}

#[test]
fn nothing_in_the_read_process_links_an_http_client() {
    for path in [
        "Cargo.toml",
        "crates/core/Cargo.toml",
        "crates/mcp/Cargo.toml",
    ] {
        let dependencies = section(path, "dependencies");
        for forbidden in ["reqwest", "ureq", "isahc", "hyper-tls"] {
            assert!(
                !dependencies.contains(forbidden),
                "{path} must not depend on {forbidden}:\n{dependencies}"
            );
        }
    }
}

#[test]
fn the_settings_type_has_nowhere_to_put_a_credential() {
    let config = source("crates/core/src/config.rs");
    let settings = config
        .split_once("pub struct McpSettings {")
        .expect("McpSettings is declared")
        .1
        .split_once('}')
        .expect("the struct closes")
        .0;
    for forbidden in ["sessionid", "token", "cookie", "session", "account"] {
        assert!(
            !settings.contains(forbidden),
            "McpSettings must have no {forbidden} field:\n{settings}"
        );
    }
}

#[test]
fn the_tools_layer_holds_no_sql_of_its_own() {
    let tools = source("crates/mcp/src/tools.rs").to_lowercase();
    for statement in ["insert into", "update ", "delete from", "select "] {
        assert!(
            !tools.contains(statement),
            "raw {statement:?} in the tools layer"
        );
    }
}

#[test]
fn the_renderer_reaches_no_database_either() {
    let render = source("crates/mcp/src/render.rs");
    assert!(
        !render.contains("sqlx") && !render.contains("PgPool"),
        "the renderer takes rows, it does not fetch them"
    );
}

#[test]
fn the_withings_crate_reaches_no_database() {
    let dependencies: String = manifest("crates/withings/Cargo.toml")
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .collect();
    assert!(
        !dependencies.contains("sqlx"),
        "the Withings client must not depend on sqlx"
    );
}
