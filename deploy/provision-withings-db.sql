-- Provision the `withings` database and its two roles on homelab-postgres.
--
-- Run as the postgres superuser:
--   psql -h homelab-postgres -U postgres \
--     -v ingest_pw="<ingest password>" -v mcp_pw="<mcp password>" \
--     -f deploy/provision-withings-db.sql
--
-- NOTE: pass RAW passwords. :'ingest_pw' quotes the value once; pre-quoting stores
-- the literal quotes in the role password and authentication then fails with a
-- message that says nothing about quoting.

CREATE ROLE withings_ingest LOGIN PASSWORD :'ingest_pw';
CREATE ROLE withings_mcp    LOGIN PASSWORD :'mcp_pw';

CREATE DATABASE withings OWNER withings_ingest;

\connect withings

GRANT ALL ON SCHEMA public TO withings_ingest;
GRANT USAGE ON SCHEMA public TO withings_mcp;

-- Redundant on PostgreSQL 15 and above, which stopped granting CREATE on `public` to
-- PUBLIC, and harmless there. Kept because the read-only rule in this repo is a risk
-- control rather than a preference, and it should not depend on the server version
-- this database happens to be restored onto.
REVOKE CREATE ON SCHEMA public FROM PUBLIC;

-- Without CREATE on the tablespaces the migration that places them fails soft and the
-- corpus silently stays on the default volume. Guarded because a host without these
-- tablespaces is a valid host.
DO $$ BEGIN
  EXECUTE 'GRANT CREATE ON TABLESPACE archive_data, nvme_indexes TO withings_ingest';
EXCEPTION WHEN undefined_object THEN
  RAISE NOTICE 'archive_data/nvme_indexes absent; the corpus will live on the default tablespace';
END $$;

-- The MCP role reads everything and writes nothing. Every table the ingester creates is
-- granted SELECT here as it is created; `ingest_session` is then revoked by 0001_init.sql
-- at the ingester's first boot, so the session jar stays unreadable to the MCP process.
--
-- No sequence grant: the schema has no serial columns, so there is no sequence to use.
ALTER DEFAULT PRIVILEGES FOR ROLE withings_ingest IN SCHEMA public
  GRANT SELECT ON TABLES TO withings_mcp;
