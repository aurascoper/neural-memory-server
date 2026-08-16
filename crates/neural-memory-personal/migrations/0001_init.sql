-- Personal memory is a separate database. Nothing here belongs in store.db.

CREATE TABLE canonical_records (
  digest          TEXT PRIMARY KEY
                  CHECK (length(digest) = 64 AND digest = lower(digest)),
  identity_domain TEXT NOT NULL CHECK (identity_domain = 'claude-mind.memory.v1'),
  content         TEXT NOT NULL,
  occurred_at     TEXT,
  metadata        TEXT NOT NULL CHECK (json_valid(metadata)),
  created_at      TEXT NOT NULL,
  tombstoned      INTEGER NOT NULL DEFAULT 0 CHECK (tombstoned IN (0, 1))
);

CREATE TABLE sightings (
  origin_device TEXT NOT NULL,
  origin_id     TEXT NOT NULL,
  record_digest TEXT NOT NULL REFERENCES canonical_records(digest),
  created_at    TEXT NOT NULL,
  source        TEXT,
  conversation  TEXT,
  PRIMARY KEY (origin_device, origin_id)
);
CREATE INDEX sightings_record_idx ON sightings(record_digest);

CREATE TABLE captures (
  origin_device TEXT NOT NULL,
  origin_id     TEXT NOT NULL,
  record_digest TEXT NOT NULL REFERENCES canonical_records(digest),
  captured_at   TEXT NOT NULL,
  PRIMARY KEY (origin_device, origin_id),
  FOREIGN KEY (origin_device, origin_id) REFERENCES sightings(origin_device, origin_id)
);

CREATE TABLE replica_records (
  record_digest TEXT PRIMARY KEY REFERENCES canonical_records(digest)
);

CREATE TABLE tags (
  name TEXT PRIMARY KEY CHECK (length(trim(name)) > 0)
);

CREATE TABLE record_tags (
  record_digest TEXT NOT NULL REFERENCES canonical_records(digest),
  tag           TEXT NOT NULL REFERENCES tags(name),
  PRIMARY KEY (record_digest, tag)
);

CREATE TABLE promotion_outbox (
  epoch         INTEGER NOT NULL,
  sequence      INTEGER NOT NULL,
  record_digest TEXT NOT NULL REFERENCES canonical_records(digest),
  enqueued_at   TEXT NOT NULL,
  status        TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'promoted')),
  promoted_at   TEXT,
  PRIMARY KEY (epoch, sequence)
);

CREATE TABLE promotion_changes (
  epoch         INTEGER NOT NULL,
  sequence      INTEGER NOT NULL,
  operation     TEXT NOT NULL CHECK (operation IN ('upsert', 'tombstone')),
  record_digest TEXT NOT NULL REFERENCES canonical_records(digest),
  PRIMARY KEY (epoch, sequence)
);

CREATE TABLE promotion_state (
  singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
  epoch     INTEGER NOT NULL CHECK (epoch >= 0),
  sequence  INTEGER NOT NULL CHECK (sequence >= 0)
);
INSERT INTO promotion_state(singleton, epoch, sequence) VALUES (1, 1, 0);

CREATE TABLE replica_cursor (
  singleton        INTEGER PRIMARY KEY CHECK (singleton = 1),
  epoch            INTEGER NOT NULL CHECK (epoch >= 0),
  sequence         INTEGER NOT NULL CHECK (sequence >= 0),
  replicated_as_of TEXT
);

INSERT INTO replica_cursor(singleton, epoch, sequence, replicated_as_of)
VALUES (1, 0, 0, NULL);
