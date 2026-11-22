# Working in this repo

## Gates — all clean before every commit

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

`cargo test` starts a `postgres:18-alpine` testcontainer, named `withings-sync-test-pg`
and marked for reuse. `docker rm -f withings-sync-test-pg` reclaims it. Tests that need
it skip with a printed reason when Docker is unreachable, so run the suite where Docker
works (on the Windows workstation that is WSL). **Never point tests at
`homelab-postgres`**: nothing in the test support reads a DSN from the environment.

## Commits

- Two lines maximum. No co-author trailers. No emojis.
- Subject line says what changed; the second line says why, if it is not obvious.

## Releases and deployment

A release is a version bump: change `version` under `[workspace.package]` in
`Cargo.toml` and merge to main. `.github/workflows/tag.yml` tags it and starts
`image.yml`, which builds `deploy/Dockerfile` and pushes `ghcr.io/rgryta/withings-sync`
as `X.Y.Z` and `latest`. The live stack follows the version tags through wud. `ci.yml`
(fmt, clippy, test, release build) runs on every pull request.

## Secrets

- `.env` files committed to this repo carry variable **names with empty values**.
- The DSN is a `Secret` (`crates/core/src/config.rs`) whose `Debug` is redacted.
- The session (the Withings `sessionid` plus the short-lived token) lives in
  `ingest_session`, which migration 0001 revokes from the MCP role. No app-embedded
  secret is used: the private API authenticates with a session token in a header.
- Never commit a captured payload. Fixtures are synthetic: call-sign words for names
  and text, `example.com` for URLs, made-up ids and plausible fake values.

## Invariants worth not breaking

- **The MCP binary never links `withings-client` or `withings-ingest`**: the process
  that serves the archive cannot reach Withings, because the code to do so is not in it.
  The root package carrying `src/bin/withings-mcp.rs` depends on `withings-core` and
  `withings-mcp-server` only, and `tests/archive/isolation.rs` keeps the manifests honest.
- **All SQL lives in `crates/core/src/store/`.** Nothing else writes a query.
- **Read-only toward Withings.** The client names read calls only (plus session refresh);
  no writes, no account updates, no push-token registration, ever.
- **The client never logs in.** A rejected session is recorded and retried after
  `WITHINGS_AUTH_RETRY`; a fresh session comes from a human in a real browser/app.
- Migrations are forward-only and never edited once applied.
- **Every profile is archived.** Unassigned weigh-ins are kept with a null person; tools
  default to the owner's profile.
