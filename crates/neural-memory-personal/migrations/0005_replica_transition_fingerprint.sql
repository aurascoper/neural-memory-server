-- Seal the last accepted record-bearing replica transition. Divergence-only
-- same-cursor updates intentionally do not replace this value.
ALTER TABLE replica_cursor ADD COLUMN transition_fingerprint TEXT
  CHECK (transition_fingerprint IS NULL OR
         (length(transition_fingerprint) = 64 AND
          transition_fingerprint = lower(transition_fingerprint)));
