# withings-sync

A permanent local archive of Withings body-measurement data, and a read-only MCP server
over it. Two binaries, one scratch image:

- `withings-ingest` reads the Withings app's private API and writes the archive in
  PostgreSQL. It backfills the whole history and keeps it current.
- `withings-mcp` serves the archive to MCP clients. It holds no credential and never
  reaches Withings.

Every profile on the account is archived; measurements are stored by type code, so a new
scale model needs no code change. The ingester keeps the session (a Withings `sessionid`
plus a short-lived token) in the `ingest_session` table, which the MCP role cannot read.

## Design notes

- **No `#` comments in `deploy/compose.yaml`.** Rationale lives here, not in the compose
  file, by house convention.
- **Raw-first storage.** Every response is kept verbatim in `raw_response`; typed tables
  (`measure`, `measure_group`, scores) are projections for the MCP to query. Nothing is
  lost when Withings adds a field.
- **Seed-once auth.** The owner signs in once; the `sessionid` is seeded into
  `ingest_session`, and the ingester refreshes the short-lived token itself. It never
  logs in and never holds the password.
