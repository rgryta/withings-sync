-- The Withings archive schema.
--
-- Raw-first: every response the ingester fetches is kept verbatim in `raw_response`,
-- and the typed tables below are projections for the MCP to query. A measure is stored
-- by its numeric type code, so a new scale model or firmware adds rows, not columns.

-- The account the session belongs to, and where an auth failure is recorded so the MCP
-- `status` tool can report it without reaching Withings.
CREATE TABLE account (
  id                  text PRIMARY KEY,
  withings_account_id bigint,
  email               text,
  timezone            text,
  last_auth_error     text,
  last_auth_error_at  timestamptz,
  updated_at          timestamptz NOT NULL DEFAULT now()
);

-- The ingester's Withings session: the persistent sessionid plus the short-lived token,
-- as JSON. Withings rotates the token on every session/renew, and the ingester persists
-- the new one here, so it lives in the database rather than the environment. The MCP
-- process must never hold a live session; the REVOKE below is the whole point.
CREATE TABLE ingest_session (
  account_id text        PRIMARY KEY,
  session    jsonb       NOT NULL,
  updated_at timestamptz NOT NULL DEFAULT now()
);
DO $$ BEGIN
  EXECUTE 'REVOKE ALL ON ingest_session FROM withings_mcp';
EXCEPTION WHEN undefined_object THEN NULL;
END $$;

-- Every profile the session can see. `id` is the Withings userid, the key every measure
-- and score carries. `is_owner` marks the account holder's own profile, which the MCP
-- tools default to. A shared or household profile is kept too, flagged by `shared`.
CREATE TABLE person (
  id         bigint PRIMARY KEY,
  label      text,
  is_owner   boolean NOT NULL DEFAULT false,
  shared     boolean NOT NULL DEFAULT false,
  raw        jsonb   NOT NULL DEFAULT '{}'::jsonb,
  updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE device (
  id         text PRIMARY KEY,
  model      text,
  raw        jsonb NOT NULL DEFAULT '{}'::jsonb,
  updated_at timestamptz NOT NULL DEFAULT now()
);

-- The names for the measure type codes the builder has identified. A code absent here is
-- still stored and shown as `type <code>`; this table only supplies a human label.
CREATE TABLE measure_type (
  code       int PRIMARY KEY,
  name       text NOT NULL,
  unit_label text
);

-- One weigh-in (or other measurement session). `deleted_at` lets a poll mark a group the
-- app no longer returns without erasing it; `category` 1 = real, 2 = target/goal.
CREATE TABLE measure_group (
  grpid       bigint PRIMARY KEY,
  person_id   bigint REFERENCES person (id),
  ts          timestamptz NOT NULL,
  device_id   text,
  model       int,
  category    smallint,
  attrib      smallint,
  modified_at timestamptz,
  deleted_at  timestamptz,
  raw         jsonb NOT NULL,
  updated_at  timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX measure_group_person_ts ON measure_group (person_id, ts DESC);
CREATE INDEX measure_group_ts ON measure_group (ts DESC);

-- One measured value. Real value = `value * 10^unit` (Withings' own encoding). `algo`
-- and `fm` are the algorithm/firmware-method provenance, kept for completeness.
CREATE TABLE measure (
  grpid     bigint NOT NULL REFERENCES measure_group (grpid) ON DELETE CASCADE,
  type_code int    NOT NULL,
  value     bigint NOT NULL,
  unit      int    NOT NULL,
  algo      int,
  fm        int,
  PRIMARY KEY (grpid, type_code)
);

-- When a value in a group changes upstream, the old one is copied here before the new
-- one overwrites it: the archive keeps history the vendor would otherwise lose.
CREATE TABLE measure_revision (
  grpid       bigint NOT NULL REFERENCES measure_group (grpid) ON DELETE CASCADE,
  type_code   int    NOT NULL,
  value       bigint NOT NULL,
  unit        int    NOT NULL,
  replaced_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX measure_revision_grp ON measure_revision (grpid, type_code);

-- Derived scores: healthindicator (vascular age, nerve health, ...) and vitality, both
-- keyed by a score kind. Stored generically so a new kind needs no migration.
CREATE TABLE indicator_score (
  id         bigint PRIMARY KEY,
  person_id  bigint REFERENCES person (id),
  source     text   NOT NULL,
  score_type text,
  ts         timestamptz NOT NULL,
  score      double precision,
  status     text,
  deleted_at timestamptz,
  raw        jsonb NOT NULL,
  updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX indicator_score_person ON indicator_score (person_id, source, ts DESC);

-- Weight-loss / trend projections.
CREATE TABLE trajectory (
  id         bigint PRIMARY KEY,
  person_id  bigint REFERENCES person (id),
  type       text,
  active     boolean,
  deleted_at timestamptz,
  raw        jsonb NOT NULL,
  updated_at timestamptz NOT NULL DEFAULT now()
);

-- The Body Scan ECG. No endpoint has been observed on this account (no watch, no scale
-- ECG recorded), so this table is future-proofing; nothing writes it yet.
CREATE TABLE ecg (
  id             bigint PRIMARY KEY,
  person_id      bigint REFERENCES person (id),
  ts             timestamptz NOT NULL,
  device_id      text,
  heart_rate     int,
  classification text,
  sampling_hz    int,
  signal         jsonb,
  deleted_at     timestamptz,
  raw            jsonb NOT NULL,
  updated_at     timestamptz NOT NULL DEFAULT now()
);

-- Every response, verbatim, keyed by endpoint kind and a natural key (a date window, a
-- userid, a group id). Lossless; a changed body overwrites.
CREATE TABLE raw_response (
  kind       text NOT NULL,
  key        text NOT NULL,
  fetched_at timestamptz NOT NULL DEFAULT now(),
  body       jsonb NOT NULL,
  PRIMARY KEY (kind, key)
);

-- One row per loop, holding its last run, the backfill cursor and the daily request
-- budget (counted in the database, so a restart does not hand out a fresh allowance).
CREATE TABLE sync_state (
  loop            text PRIMARY KEY,
  last_run_at     timestamptz,
  last_ok_at      timestamptz,
  last_error      text,
  budget_day      date,
  budget_spent    bigint NOT NULL DEFAULT 0,
  backfill_floor  timestamptz,
  backfill_cursor timestamptz,
  backfill_done   boolean NOT NULL DEFAULT false
);

-- A measure read as its real numeric value, named where the type is known. A left join
-- keeps unknown codes (null name), so nothing is hidden by an unseeded type.
CREATE VIEW measure_value AS
SELECT g.grpid,
       g.person_id,
       g.ts,
       m.type_code,
       t.name AS type_name,
       trim_scale((m.value::numeric) * power(10::numeric, m.unit)) AS value,
       g.deleted_at IS NOT NULL AS deleted
FROM measure_group g
JOIN measure m ON m.grpid = g.grpid
LEFT JOIN measure_type t ON t.code = m.type_code;

-- The measure type codes known today, from the Withings public meastype table. Unknown
-- codes still store and render as `type <code>`; this only adds a friendly name.
INSERT INTO measure_type (code, name, unit_label) VALUES
  (1,  'weight',                  'kg'),
  (4,  'height',                  'm'),
  (5,  'fat_free_mass',           'kg'),
  (6,  'fat_ratio',               '%'),
  (8,  'fat_mass',                'kg'),
  (9,  'diastolic_blood_pressure','mmHg'),
  (10, 'systolic_blood_pressure', 'mmHg'),
  (11, 'heart_pulse',             'bpm'),
  (12, 'temperature',             'C'),
  (54, 'spo2',                    '%'),
  (71, 'body_temperature',        'C'),
  (73, 'skin_temperature',        'C'),
  (76, 'muscle_mass',             'kg'),
  (77, 'hydration',               'kg'),
  (88, 'bone_mass',               'kg'),
  (91, 'pulse_wave_velocity',     'm/s'),
  (122,'visceral_fat',            ''),
  (155,'vascular_age',            'years'),
  (170,'visceral_fat_index',      ''),
  (226,'basal_metabolic_rate',    'kcal'),
  (227,'metabolic_age',           'years')
ON CONFLICT (code) DO NOTHING;
