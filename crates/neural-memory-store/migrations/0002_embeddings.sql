-- M2 vector branch.
--
-- No index. Deliberately: at this corpus size a brute-force scan of a few
-- thousand 768-float vectors is a few milliseconds and about 15 MB, while an
-- ANN index would add an extension dependency, a build step, and approximate
-- results -- all to optimise something that is not the bottleneck. When a scan
-- stops being fast enough, `tests/vector.rs` measures it and will say so.
--
-- `embedding_profiles` already exists from 0001 as a claude-mind compatibility
-- shim, with a test asserting it stayed empty through M1. It now becomes real.

ALTER TABLE embedding_profiles ADD COLUMN identity TEXT;
ALTER TABLE embedding_profiles ADD COLUMN model_revision TEXT;
ALTER TABLE embedding_profiles ADD COLUMN pooling TEXT;
ALTER TABLE embedding_profiles ADD COLUMN normalization TEXT;
ALTER TABLE embedding_profiles ADD COLUMN task_instruction TEXT;
ALTER TABLE embedding_profiles ADD COLUMN weight_sha256 TEXT;
ALTER TABLE embedding_profiles ADD COLUMN tokenizer_sha256 TEXT;

-- Unconditional, not partial: SQLite will not accept a PARTIAL unique index as
-- a foreign-key target, so `embeddings.profile_identity` could not reference it.
-- Safe because M1 asserts this table is empty, so there are no pre-existing
-- NULL rows to collide.
CREATE UNIQUE INDEX embedding_profiles_identity_idx ON embedding_profiles(identity);

-- One vector per (space, record). The primary key is the index-sharing law
-- made structural: a record can hold a vector in many spaces at once, and no
-- query can accidentally compare across them because the space is part of the
-- key it selects on.
CREATE TABLE embeddings (
  profile_identity TEXT NOT NULL REFERENCES embedding_profiles(identity),
  record_digest    TEXT NOT NULL REFERENCES memories(record_digest) ON DELETE CASCADE,

  -- Little-endian f32, `dimensions * 4` bytes. Checked on read against the
  -- profile's declared dimensions -- a truncated blob would otherwise produce
  -- a plausible cosine rather than an error.
  vector           BLOB NOT NULL,

  -- The exact text embedded. NOT the claim: the task instruction prefix is part
  -- of what was fed to the model, and reconstructing it later from the profile
  -- would be guessing at the thing the space identity says matters.
  embedded_text    TEXT NOT NULL,
  embedded_at      TEXT NOT NULL,

  PRIMARY KEY (profile_identity, record_digest)
);

CREATE INDEX embeddings_profile_idx ON embeddings(profile_identity);
