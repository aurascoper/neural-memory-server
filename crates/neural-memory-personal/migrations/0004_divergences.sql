-- Explicit personal divergence. Neither side is selected, changed, or retired.
CREATE TABLE personal_divergences (
  id              TEXT PRIMARY KEY
                  CHECK (length(id) = 64 AND id = lower(id)),
  digest_a        TEXT NOT NULL REFERENCES canonical_records(digest),
  digest_b        TEXT NOT NULL REFERENCES canonical_records(digest),
  status          TEXT NOT NULL CHECK (status IN ('unacknowledged', 'acknowledged')),
  created_at      TEXT NOT NULL,
  acknowledged_at TEXT,
  UNIQUE (digest_a, digest_b),
  CHECK (digest_a < digest_b),
  CHECK ((status = 'unacknowledged' AND acknowledged_at IS NULL)
      OR (status = 'acknowledged' AND acknowledged_at IS NOT NULL))
);

CREATE INDEX personal_divergences_status_idx
  ON personal_divergences(status, digest_a, digest_b);
