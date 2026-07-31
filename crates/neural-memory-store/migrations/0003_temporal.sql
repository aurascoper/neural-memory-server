-- Transaction time made queryable.
--
-- `recorded_seq` was already a monotonic transaction-time axis. What was
-- missing is the ability to ask when a RETIREMENT happened in that same order:
-- `superseded_at` is a wall-clock string, and comparing it against a sequence
-- is not a comparison at all. Reconstructing belief at a past point needs both
-- endpoints on one axis.

ALTER TABLE memories ADD COLUMN superseded_seq INTEGER;
ALTER TABLE memories ADD COLUMN retracted_seq  INTEGER;

-- Wall-clock transaction time, for resolving "what did we believe on the 30th"
-- into a sequence. Deliberately NULLABLE and deliberately NOT backfilled:
-- records written before this migration have a transaction-time ORDERING but no
-- known instant, and copying `occurred_at` into it would conflate the two axes
-- this migration exists to separate. `seq_at()` reports that gap rather than
-- inventing a timestamp.
ALTER TABLE memories ADD COLUMN recorded_at TEXT;

-- Backfill for retirements that already happened. A supersession cannot have
-- been recorded before its replacement existed, so the replacement's own
-- sequence is a sound LOWER BOUND -- the earliest point at which the store
-- could have known. It is an approximation, and it is one in the safe
-- direction: a belief reconstruction may show a claim as still-current
-- slightly too long, never as retired before anyone could have retired it.
UPDATE memories
   SET superseded_seq = (
        SELECT r.recorded_seq FROM memories r
         WHERE r.record_digest = memories.superseded_by)
 WHERE superseded_by IS NOT NULL AND superseded_seq IS NULL;

CREATE INDEX memories_superseded_seq_idx ON memories(superseded_seq);
CREATE INDEX memories_recorded_at_idx    ON memories(recorded_at);

-- A retirement is recorded at or after the record it retires. Violating this
-- would mean believing a claim was retired before it was written.
CREATE TRIGGER memories_supersede_seq_sane
BEFORE UPDATE OF superseded_seq ON memories
WHEN new.superseded_seq IS NOT NULL AND new.superseded_seq < new.recorded_seq
BEGIN
  SELECT RAISE(ABORT, 'superseded_seq precedes recorded_seq: a claim cannot be retired before it was written');
END;
