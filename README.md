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

## Seeding the session

The ingester never does the interactive sign-in. Seed it once with the **device
credentials** a signed-in Withings client holds — these do not expire and need no 2FA
code, so the ingester re-authenticates itself from then on (via `auth?action=login`):

1. Capture a session JSON from a signed-in client's `auth?action=login` request:
   `{ "auth_token": "...", "providertoken": "...", "device_token": "...", "uuid": "...",
   "token": "...", "accountid": 0, "owner_userid": 0 }`. The four device credentials are
   the persistent ones; `token` is re-minted automatically.
2. Put it on the host and point the ingester at it for one start with
   `WITHINGS_SESSION_FROM=/path/to/session.json`. It is written to `ingest_session` and the
   database wins from then on; remove the env var and file afterwards.

To replace dead device credentials, delete the `ingest_session` row and seed again.
