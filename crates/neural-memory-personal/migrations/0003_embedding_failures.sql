-- Terminal, record-local derivation failures. These are local and never sync.
CREATE TABLE personal_embedding_failures (
  profile_identity TEXT NOT NULL REFERENCES personal_embedding_profiles(identity),
  record_digest    TEXT NOT NULL REFERENCES canonical_records(digest),
  reason           TEXT NOT NULL CHECK (reason IN ('input-too-large')),
  failed_at        TEXT NOT NULL,
  PRIMARY KEY (profile_identity, record_digest)
);
