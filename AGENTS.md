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
- The session lives in `ingest_session`, which migration 0001 revokes from the MCP role.
  It holds the persistent device credentials (`auth_token`, `providertoken`,
  `device_token`, `uuid`) the ingester re-authenticates with via `auth?action=login` —
  no 2FA code or user interaction needed — plus the current session token. The private
  API authenticates each call with that token in a `Session-Id` header.
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
- **The client never does the interactive sign-in** (email shortcode + 2FA): it re-auths
  silently from the stored device credentials. A rejected session is recorded and retried after
  `WITHINGS_AUTH_RETRY`; a fresh session comes from a human in a real browser/app.
- Migrations are forward-only and never edited once applied.
- **Every profile is archived.** Unassigned weigh-ins are kept with a null person; tools
  default to the owner's profile.
