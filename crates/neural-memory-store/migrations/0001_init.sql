-- neural-memory-server, migration 0001.
--
-- Table SHAPE is carried over from claude-mind-mcp's sql/pgvector_schema.sql
-- (memories, entities, mentions, relations, tags, memory_tags,
-- embedding_profiles), translated to SQLite. Postgres was dropped once the
-- latency argument was falsified: with no vector branch in M1 and a corpus of a
-- few hundred records, a daemon buys nothing that FTS5 and WITH RECURSIVE do not
-- already provide.
--
-- Type translation: UUID -> TEXT, TIMESTAMPTZ -> TEXT (RFC-3339 UTC),
-- JSONB -> TEXT with CHECK (json_valid(...)), TSVECTOR -> an FTS5 table.
--
-- There are NO UUIDs anywhere in this schema. A record's identity IS its
-- content digest. Upstream's rule -- "the primary key is a foreign-key target,
-- the 64-hex digest is the identity" -- is satisfied here by removing the
-- distinction rather than policing it: if no UUID exists, none can be mistaken
-- for an identity.

-- ---------------------------------------------------------------------------
-- Claims
-- ---------------------------------------------------------------------------

CREATE TABLE memories (
  -- Monotonic and gapless-by-insertion. THIS IS transaction time: "what did we
  -- believe at time T" is "scan to seq N". That is why the plan carries no
  -- bitemporal machinery -- an append-only log with a sequence already is one
  -- axis, and hand-rolled tstzrange joins would buy nothing here.
  recorded_seq      INTEGER PRIMARY KEY AUTOINCREMENT,

  -- The seal. Also the idempotency key: importing the same claim twice is one
  -- row and no new history.
  record_digest     TEXT NOT NULL UNIQUE
                    CHECK (length(record_digest) = 64 AND record_digest = lower(record_digest)),

  claim             TEXT NOT NULL CHECK (length(trim(claim)) > 0),

  -- Derived from verifiable structure by the write path, never accepted from a
  -- caller. The agent-facing MCP surface can only ever emit 'agentInference'.
  evidence_class    TEXT NOT NULL CHECK (evidence_class IN
                      ('observed','derivedDeterministically','humanDecision',
                       'agentInference','externalClaim')),

  source_artifact_sha256 TEXT REFERENCES artifacts(sha256_hex),
  source_locator    TEXT,
  harness_run_id    TEXT,
  runtime_identity  TEXT,

  -- Valid time: when the claim was true, as distinct from when it was recorded.
  occurred_at       TEXT,

  -- Retirement. A superseded record is NEVER deleted and never edited in place:
  -- default retrieval hides it, explicit retrieval still returns it, and the
  -- assembler leaves it in any prefix that already showed it.
  superseded_by     TEXT REFERENCES memories(record_digest),
  superseded_at     TEXT,
  retracted_at      TEXT,
  retraction_reason TEXT,

  metadata          TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(metadata)),

  -- A record cannot supersede itself, and a retirement must say when.
  CHECK (superseded_by IS NULL OR superseded_by <> record_digest),
  CHECK ((superseded_by IS NULL) = (superseded_at IS NULL)),
  CHECK ((retracted_at IS NULL) = (retraction_reason IS NULL))
);

CREATE INDEX memories_superseded_idx ON memories(superseded_at);
CREATE INDEX memories_occurred_idx   ON memories(occurred_at DESC);
CREATE INDEX memories_class_idx      ON memories(evidence_class);

-- ---------------------------------------------------------------------------
-- Lexical retrieval
-- ---------------------------------------------------------------------------

-- External-content FTS5: the text lives in `memories`, this holds only the
-- index. bm25(fts, w_claim, w_locator) recovers most of what Postgres setweight
-- gave us.
CREATE VIRTUAL TABLE memories_fts USING fts5(
  claim,
  source_locator,
  content='memories',
  content_rowid='recorded_seq',
  tokenize='porter unicode61'
);

CREATE TRIGGER memories_fts_ai AFTER INSERT ON memories BEGIN
  INSERT INTO memories_fts(rowid, claim, source_locator)
  VALUES (new.recorded_seq, new.claim, coalesce(new.source_locator, ''));
END;

CREATE TRIGGER memories_fts_ad AFTER DELETE ON memories BEGIN
  INSERT INTO memories_fts(memories_fts, rowid, claim, source_locator)
  VALUES ('delete', old.recorded_seq, old.claim, coalesce(old.source_locator, ''));
END;

CREATE TRIGGER memories_fts_au AFTER UPDATE OF claim, source_locator ON memories BEGIN
  INSERT INTO memories_fts(memories_fts, rowid, claim, source_locator)
  VALUES ('delete', old.recorded_seq, old.claim, coalesce(old.source_locator, ''));
  INSERT INTO memories_fts(rowid, claim, source_locator)
  VALUES (new.recorded_seq, new.claim, coalesce(new.source_locator, ''));
END;

-- ---------------------------------------------------------------------------
-- Evidence: artifacts -> observations -> claims
-- ---------------------------------------------------------------------------

CREATE TABLE artifacts (
  sha256_hex    TEXT PRIMARY KEY
                CHECK (length(sha256_hex) = 64 AND sha256_hex = lower(sha256_hex)),
  artifact_kind TEXT NOT NULL,
  byte_size     INTEGER NOT NULL CHECK (byte_size >= 0),
  media_type    TEXT NOT NULL,
  source_uri    TEXT NOT NULL,
  ingested_at   TEXT NOT NULL
);

CREATE TABLE measurement_policies (
  identity        TEXT PRIMARY KEY CHECK (length(identity) = 64),
  -- The field whose absence upstream is the defect this store exists to prevent.
  -- `max_logit_divergence` there is a bare f64: cosine? max-abs? L2? RMS?
  -- Unanswerable, so two labs could both read Conformant having measured
  -- different quantities.
  metric          TEXT NOT NULL CHECK (length(trim(metric)) > 0),
  aggregation     TEXT NOT NULL CHECK (length(trim(aggregation)) > 0),
  comparison_rule TEXT NOT NULL CHECK (length(trim(comparison_rule)) > 0),
  step_budget     INTEGER CHECK (step_budget IS NULL OR step_budget > 0),
  unit            TEXT NOT NULL
);

CREATE TABLE evaluation_suites (
  identity          TEXT PRIMARY KEY CHECK (length(identity) = 64),
  suite_name        TEXT NOT NULL,
  -- Sorted JSON array. Replaces the generation contract's SINGULAR
  -- prompt_byte_identity, which cannot describe a 20-50 prompt corpus.
  case_digests      TEXT NOT NULL CHECK (json_valid(case_digests)),
  tokenizer_identity TEXT NOT NULL,
  context_cap       INTEGER NOT NULL CHECK (context_cap > 0)
);

CREATE TABLE reference_executions (
  identity                 TEXT PRIMARY KEY CHECK (length(identity) = 64),
  runtime_identity         TEXT NOT NULL,
  backend_id               TEXT NOT NULL,
  artifact_sha256          TEXT NOT NULL REFERENCES artifacts(sha256_hex),
  evaluation_suite_identity TEXT NOT NULL REFERENCES evaluation_suites(identity),
  environment              TEXT NOT NULL CHECK (json_valid(environment))
);

CREATE TABLE observations (
  identity                  TEXT PRIMARY KEY CHECK (length(identity) = 64),
  observation_kind          TEXT NOT NULL,

  quantity_kind             TEXT NOT NULL CHECK (quantity_kind IN ('absolute','relative')),

  -- Canonical decimal TEXT, not REAL. A float in a sealed document is at the
  -- mercy of the serializer's formatter; the sealed form is the exact text
  -- measured. `value_real` is a derived convenience for range queries and is
  -- never the thing hashed.
  value_text                TEXT NOT NULL,
  value_real                REAL,

  measurement_policy_identity TEXT NOT NULL REFERENCES measurement_policies(identity),
  evaluation_suite_identity   TEXT NOT NULL REFERENCES evaluation_suites(identity),
  reference_execution_identity TEXT REFERENCES reference_executions(identity),

  runtime_identity          TEXT NOT NULL,
  artifact_sha256           TEXT REFERENCES artifacts(sha256_hex),
  observed_at               TEXT NOT NULL,

  -- THE constraint. A relative quantity -- a divergence, a speedup, a delta --
  -- with nothing to be relative to is not a measurement. Storing the GPD
  -- observations without this populated would faithfully reproduce the very
  -- defect the store exists to document.
  CONSTRAINT observation_relative_needs_reference
    CHECK (quantity_kind <> 'relative' OR reference_execution_identity IS NOT NULL),

  -- Scope cuts both ways, mirroring MeasurementOutOfScope upstream: an
  -- out-of-scope field is visible, not silently ignored.
  CONSTRAINT observation_absolute_forbids_reference
    CHECK (quantity_kind <> 'absolute' OR reference_execution_identity IS NULL)
);

CREATE INDEX observations_kind_idx ON observations(observation_kind);
CREATE INDEX observations_ref_idx  ON observations(reference_execution_identity);

CREATE TABLE memory_observations (
  record_digest        TEXT NOT NULL REFERENCES memories(record_digest) ON DELETE CASCADE,
  observation_identity TEXT NOT NULL REFERENCES observations(identity) ON DELETE CASCADE,
  PRIMARY KEY (record_digest, observation_identity)
);

-- ---------------------------------------------------------------------------
-- Provenance graph
-- ---------------------------------------------------------------------------

CREATE TABLE provenance_edges (
  src_digest TEXT NOT NULL,
  dst_digest TEXT NOT NULL,
  edge_kind  TEXT NOT NULL CHECK (edge_kind IN
               ('supersedes','derivedFrom','supports','contradicts','citesArtifact')),
  created_at TEXT NOT NULL,
  PRIMARY KEY (src_digest, dst_digest, edge_kind),
  CHECK (src_digest <> dst_digest)
);

CREATE INDEX provenance_src_idx ON provenance_edges(src_digest);
CREATE INDEX provenance_dst_idx ON provenance_edges(dst_digest);

-- ---------------------------------------------------------------------------
-- Entities (tables exist in M1; the entity retrieval branch lands in M2)
-- ---------------------------------------------------------------------------

CREATE TABLE entities (
  id             TEXT PRIMARY KEY,
  canonical_name TEXT NOT NULL,
  entity_type    TEXT NOT NULL,
  aliases        TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(aliases))
);
CREATE INDEX entities_name_idx ON entities(canonical_name);

CREATE TABLE mentions (
  record_digest TEXT NOT NULL REFERENCES memories(record_digest) ON DELETE CASCADE,
  entity_id     TEXT NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
  start_offset  INTEGER NOT NULL CHECK (start_offset >= 0),
  end_offset    INTEGER NOT NULL,
  extractor_identity TEXT NOT NULL,
  PRIMARY KEY (record_digest, entity_id, start_offset),
  CHECK (end_offset > start_offset)
);

CREATE TABLE relations (
  id                 TEXT PRIMARY KEY,
  subject_entity_id  TEXT NOT NULL REFERENCES entities(id),
  predicate          TEXT NOT NULL,
  object_entity_id   TEXT NOT NULL REFERENCES entities(id),
  provenance_digest  TEXT NOT NULL REFERENCES memories(record_digest),
  valid_from         TEXT,
  valid_to           TEXT
);

CREATE TABLE tags (
  id   TEXT PRIMARY KEY,
  name TEXT NOT NULL UNIQUE
);

CREATE TABLE memory_tags (
  record_digest TEXT NOT NULL REFERENCES memories(record_digest) ON DELETE CASCADE,
  tag_id        TEXT NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
  PRIMARY KEY (record_digest, tag_id)
);

-- Kept for interchange with claude-mind-mcp. M1 declares no embedding space and
-- a test asserts this table is EMPTY, so its emptiness is a claim rather than an
-- accident: an unused table is an affordance, and someone would otherwise record
-- a profile that was never used with nothing objecting.
CREATE TABLE embedding_profiles (
  id         TEXT PRIMARY KEY,
  backend    TEXT NOT NULL,
  model_name TEXT NOT NULL,
  dim        INTEGER NOT NULL,
  seq_len    INTEGER NOT NULL,
  created_at TEXT NOT NULL
);

-- ---------------------------------------------------------------------------
-- Sessions: what the assembler has already emitted
-- ---------------------------------------------------------------------------

CREATE TABLE sessions (
  id                    TEXT PRIMARY KEY,
  started_at            TEXT NOT NULL,
  context_budget_tokens INTEGER NOT NULL CHECK (context_budget_tokens > 0)
);

-- Append-only by construction: `position` is unique per session and no update
-- path exists. Re-emitting a record the session has already seen is prevented by
-- the (session_id, record_digest) uniqueness, not by convention.
CREATE TABLE session_emissions (
  session_id    TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  position      INTEGER NOT NULL,
  record_digest TEXT NOT NULL REFERENCES memories(record_digest),
  turn          INTEGER NOT NULL,
  token_cost    INTEGER NOT NULL CHECK (token_cost >= 0),
  PRIMARY KEY (session_id, position),
  UNIQUE (session_id, record_digest)
);

-- `schema_migrations` is deliberately NOT declared here: the migration runner
-- creates and owns it before any migration runs, so declaring it in 0001 would
-- make the very first migration fail against its own bookkeeping table.
