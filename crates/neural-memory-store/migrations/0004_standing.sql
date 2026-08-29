-- Standing records: claims that are IN FORCE, not merely recallable.
--
-- Everything in this store is inert until a tool call names it. That is correct
-- for evidence and wrong for a rule: a rule learned after the commit has already
-- landed did not govern anything. The motivating case was a git-worktree
-- discipline that lived in a plan file, was not in force for the next session,
-- and let the same mistake happen twice -- while the store could not even answer
-- "was that rule in force on the 28th", because it held no notion of in force.
--
-- Two columns, mirroring the superseded_at/superseded_seq pair 0003 established,
-- and for the identical reason. `standing_from` is wall clock: when it began to
-- bind in the world. `standing_seq` is the transaction-order point. Both are
-- needed because a record can be declared standing long after it was written, so
-- `recorded_seq <= ?` would answer the historical question far too permissively.
--
-- OUTSIDE THE SEAL, exactly as retirement is. `memory_record_identity` hashes
-- what is CLAIMED. Whether the store treats a claim as binding is its posture
-- toward the record, not a term of the claim. Were standing sealed, declaring a
-- rule standing would mint a second digest for the same claim, the original
-- would remain un-retired, and `recall` would return two records asserting the
-- same thing with different force.
--
-- Not `metadata`: no query reads it, and the one field gating every future
-- session does not belong in an unindexed JSON blob with no closed-set
-- constraint. Not `tags`: `tags.name` is free text, so `standing`, `Standing`
-- and `standing-rule` would all be distinct and nothing would object --
-- reintroducing the second mutable addressing scheme 0001 removed by
-- construction. A nullable timestamp is a closed set of two states.

ALTER TABLE memories ADD COLUMN standing_from TEXT;
ALTER TABLE memories ADD COLUMN standing_seq  INTEGER;

CREATE INDEX memories_standing_seq_idx ON memories(standing_seq);

-- RETIREMENT DOMINATES, once, in the schema.
--
-- retrieval.rs repeats the retirement predicate by hand at four sites, and three
-- of them had it half-written for as long as the column existed. A fifth Rust
-- `if` would be a fifth place to forget. A view is not a fifth site: it is the
-- predicate ONCE, unreachable by a caller who forgets it. Anyone writing
-- `SELECT ... FROM memories WHERE standing_from IS NOT NULL` is writing a query
-- with no caller and no test.
--
-- ORDER BY standing_seq is load-bearing, not cosmetic: declarations append, so
-- the injected block is a stable prefix across sessions. Ordering by digest --
-- what assembler.rs::plan_append does -- would reshuffle the whole block every
-- time a rule was added, and a prefix that reshuffles is not a prefix.
CREATE VIEW standing_current AS
  SELECT recorded_seq, record_digest, claim, evidence_class,
         standing_from, standing_seq, occurred_at, recorded_at
    FROM memories
   WHERE standing_from IS NOT NULL
     AND superseded_at IS NULL
     AND retracted_at  IS NULL
   ORDER BY standing_seq, recorded_seq;

-- ONLY A humanDecision MAY BIND.
--
-- The argument is the one already written at mcp/src/lib.rs:21-25, a fortiori.
-- `supersede` is kept out of agent reach because an agent that can be
-- prompt-injected into retiring true records fails worse than one that cannot
-- retire anything. Standing is strictly stronger: it inserts text into every
-- future session before any work begins. An agent talked into declaring
-- "sessions share one checkout" as standing has not merely corrupted the store,
-- it has installed the corruption ahead of every future reader, human included.
--
-- No new clamp is needed, and that is the point. write.rs forces the agent
-- channel to 'agentInference' BEFORE the digest is computed, so an agent cannot
-- produce the bytes of a humanDecision record -- it cannot mint one that is even
-- ELIGIBLE. This trigger keys on evidence_class rather than on a write channel
-- because the channel is invisible in SQL while its consequence is not, and
-- because a class check cannot drift out of sync with the clamp that sets it.
--
-- Both polarities, per 0001's stated doctrine that a rule living in one place is
-- bypassable through the other. put_memory never inserts standing_from today;
-- the INSERT trigger costs four lines and closes the door before anyone opens it.
CREATE TRIGGER memories_standing_requires_human_decision_ins
BEFORE INSERT ON memories
WHEN new.standing_from IS NOT NULL AND new.evidence_class <> 'humanDecision'
BEGIN
  SELECT RAISE(ABORT, 'standing requires humanDecision: a rule that binds every future session may not be an inference');
END;

CREATE TRIGGER memories_standing_requires_human_decision_upd
BEFORE UPDATE OF standing_from ON memories
WHEN new.standing_from IS NOT NULL AND new.evidence_class <> 'humanDecision'
BEGIN
  SELECT RAISE(ABORT, 'standing requires humanDecision: a rule that binds every future session may not be an inference');
END;

-- The two columns are one fact and must not disagree. SQLite cannot add a
-- table-level CHECK by ALTER TABLE, so this is a trigger for the same reason
-- memories_supersede_seq_sane is.
CREATE TRIGGER memories_standing_pair_ins
BEFORE INSERT ON memories
WHEN (new.standing_from IS NULL) <> (new.standing_seq IS NULL)
BEGIN
  SELECT RAISE(ABORT, 'standing_from and standing_seq must be set together: one without the other is half an interval');
END;

CREATE TRIGGER memories_standing_pair_upd
BEFORE UPDATE OF standing_from, standing_seq ON memories
WHEN (new.standing_from IS NULL) <> (new.standing_seq IS NULL)
BEGIN
  SELECT RAISE(ABORT, 'standing_from and standing_seq must be set together: one without the other is half an interval');
END;

-- A rule cannot bind before it was written. Sibling of
-- memories_supersede_seq_sane, and violating it would make a belief
-- reconstruction report a rule as governing a session that predates it.
CREATE TRIGGER memories_standing_seq_sane
BEFORE UPDATE OF standing_seq ON memories
WHEN new.standing_seq IS NOT NULL AND new.standing_seq < new.recorded_seq
BEGIN
  SELECT RAISE(ABORT, 'standing_seq precedes recorded_seq: a rule cannot bind before it was written');
END;

-- CONTEXT IS NOT FREE.
--
-- Standing records are a permanent per-session tax: every one of them is paid
-- for in every future session, forever. An unbounded set eventually crowds out
-- the work it exists to protect.
--
-- Enforced at DECLARATION, at the operator's desk, where something can be
-- retired to make room -- never at emission. Truncating at emission would fail
-- open on precisely the rules it dropped, silently, in the moment they were
-- needed. Refusal here cannot be reached behind the trigger's back either:
-- claims are sealed, so a declared set cannot grow by editing.
--
-- 6000 characters is roughly 1500 tokens. The char count is a deliberate proxy
-- for a tokenizer, and it is a CEILING rather than an estimate. The Rust layer
-- re-checks and lists the current set with sizes, because a refusal that does
-- not say what is already competing forces a separate query at the one moment
-- someone needs to choose what to retire.
CREATE TRIGGER memories_standing_budget
BEFORE UPDATE OF standing_from ON memories
WHEN new.standing_from IS NOT NULL
 AND (SELECT coalesce(sum(length(claim)), 0) FROM standing_current)
     + length(new.claim) > 6000
BEGIN
  SELECT RAISE(ABORT, 'standing set would exceed its budget: retire a rule before adding one');
END;
