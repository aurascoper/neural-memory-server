-- Local-only personal embeddings. Vectors never participate in sync.
CREATE TABLE personal_embedding_profiles (
  identity       TEXT PRIMARY KEY CHECK (length(identity) = 64 AND identity = lower(identity)),
  backend        TEXT NOT NULL,
  model_artifact TEXT NOT NULL,
  dimension      INTEGER NOT NULL CHECK (dimension > 0 AND dimension <= 65536),
  normalization  TEXT NOT NULL CHECK (normalization IN ('l2', 'none')),
  version        TEXT NOT NULL,
  adapter        TEXT NOT NULL CHECK (adapter IN ('llama-cpp-http', 'deterministic-test')),
  endpoint       TEXT
);

CREATE TABLE personal_active_embedding_profile (
  singleton        INTEGER PRIMARY KEY CHECK (singleton = 1),
  profile_identity TEXT NOT NULL REFERENCES personal_embedding_profiles(identity)
);

CREATE TABLE personal_embeddings (
  profile_identity TEXT NOT NULL REFERENCES personal_embedding_profiles(identity),
  record_digest    TEXT NOT NULL REFERENCES canonical_records(digest),
  vector           BLOB NOT NULL,
  embedded_at      TEXT NOT NULL,
  PRIMARY KEY (profile_identity, record_digest)
);

CREATE TABLE personal_embedding_queue (
  record_digest    TEXT PRIMARY KEY REFERENCES canonical_records(digest),
  profile_identity TEXT REFERENCES personal_embedding_profiles(identity),
  enqueued_at      TEXT NOT NULL
);
