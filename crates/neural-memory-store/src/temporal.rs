//! Two time axes, and belief reconstruction across them.
//!
//! See `docs/temporal-queries.md` for the three queries that justify this
//! existing at all; it was written before the migration, so the design answers a
//! stated need rather than the need being invented to fit it.
//!
//! - **Valid time** — when a claim was true of the world (`occurred_at`).
//! - **Transaction time** — when the store came to believe it (`recorded_seq`,
//!   monotonic, no clock involved).
//!
//! What is deliberately absent: `tstzrange`, exclusion constraints, Allen
//! relations. Those three queries need a sequence comparison, and that is what
//! they get.

use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::{Store, StoreError};

/// Resolving a wall-clock instant to a point in transaction order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum SeqAt {
    /// The instant resolves cleanly: this is the last sequence recorded at or
    /// before it.
    Resolved { seq: i64 },
    /// The store held nothing that early.
    BeforeAnyRecord,
    /// Records exist that early, but they predate migration 0003 and carry no
    /// `recorded_at`. Their transaction time is known as an ORDERING only.
    /// Reported rather than guessed: inventing a timestamp for them would
    /// fabricate exactly the precision the two-axis split exists to avoid.
    UnknownBefore { earliest_known_seq: Option<i64> },
}

/// A record's status as of some point in transaction time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BeliefAt {
    /// Written by then and not yet retired.
    Current,
    /// Written by then, and already retired by then.
    Retired,
    /// Not yet written.
    NotYetKnown,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineEntry {
    pub recorded_seq: i64,
    pub record_digest: String,
    pub claim: String,
    pub evidence_class: String,
    pub recorded_at: Option<String>,
    pub occurred_at: Option<String>,
    /// Sequence at which this was retired, if it has been.
    pub superseded_seq: Option<i64>,
    pub superseded_by: Option<String>,
}

impl Store {
    /// Last sequence recorded at or before `instant`.
    pub fn seq_at(&self, instant: &str) -> Result<SeqAt, StoreError> {
        let resolved: Option<i64> = self.conn.query_row(
            "SELECT max(recorded_seq) FROM memories
             WHERE recorded_at IS NOT NULL AND recorded_at <= ?1",
            params![instant],
            |r| r.get(0),
        )?;
        if let Some(seq) = resolved {
            return Ok(SeqAt::Resolved { seq });
        }
        // Nothing timestamped that early. Distinguish "the store was empty" from
        // "the early records simply have no timestamp".
        let untimed: Option<i64> = self.conn.query_row(
            "SELECT min(recorded_seq) FROM memories WHERE recorded_at IS NULL",
            [],
            |r| r.get(0),
        )?;
        Ok(match untimed {
            Some(seq) => SeqAt::UnknownBefore {
                earliest_known_seq: Some(seq),
            },
            None => SeqAt::BeforeAnyRecord,
        })
    }

    /// What the store believed about one record at a point in transaction time.
    pub fn belief_at(&self, digest: &str, seq: i64) -> Result<Option<BeliefAt>, StoreError> {
        let row: Option<(i64, Option<i64>, Option<i64>)> = self
            .conn
            .query_row(
                "SELECT recorded_seq, superseded_seq, retracted_seq
                 FROM memories WHERE record_digest = ?1",
                params![digest],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .ok();
        Ok(row.map(|(rec, sup, ret)| {
            if rec > seq {
                BeliefAt::NotYetKnown
            } else if sup.is_some_and(|s| s <= seq) || ret.is_some_and(|s| s <= seq) {
                BeliefAt::Retired
            } else {
                BeliefAt::Current
            }
        }))
    }

    /// Everything the store held as current at a point in transaction time.
    ///
    /// A record retired *after* `seq` counts as current, which is the whole
    /// point: reconstructing a past belief means ignoring corrections that had
    /// not happened yet.
    pub fn current_as_of(&self, seq: i64, limit: usize) -> Result<Vec<TimelineEntry>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT recorded_seq, record_digest, claim, evidence_class,
                    recorded_at, occurred_at, superseded_seq, superseded_by
             FROM memories
             WHERE recorded_seq <= ?1
               AND (superseded_seq IS NULL OR superseded_seq > ?1)
               AND (retracted_seq  IS NULL OR retracted_seq  > ?1)
             ORDER BY recorded_seq
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![seq, limit as i64], |r| {
            Ok(TimelineEntry {
                recorded_seq: r.get(0)?,
                record_digest: r.get(1)?,
                claim: r.get(2)?,
                evidence_class: r.get(3)?,
                recorded_at: r.get(4)?,
                occurred_at: r.get(5)?,
                superseded_seq: r.get(6)?,
                superseded_by: r.get(7)?,
            })
        })?;
        let mut out = Vec::new();
        for x in rows {
            out.push(x?);
        }
        Ok(out)
    }

    /// Q3: was the evidence this claim cites already retired when it was made?
    ///
    /// A claim resting on evidence retired *before* it existed is a defect. One
    /// resting on evidence retired *afterwards* is ordinary history. Only a
    /// comparison of two transaction times tells them apart.
    pub fn cited_stale_evidence(
        &self,
        digest: &str,
    ) -> Result<Vec<(String, i64, i64)>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT src.record_digest, me.recorded_seq, src.superseded_seq
             FROM memories me
             JOIN provenance_edges e ON e.src_digest = me.record_digest
             JOIN memories src ON src.record_digest = e.dst_digest
             WHERE me.record_digest = ?1
               AND src.superseded_seq IS NOT NULL
               AND src.superseded_seq < me.recorded_seq
             ORDER BY src.record_digest",
        )?;
        let rows = stmt.query_map(params![digest], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        let mut out = Vec::new();
        for x in rows {
            out.push(x?);
        }
        Ok(out)
    }

    /// Q2: records whose VALID time falls in a window, restricted to what was
    /// known by a point in TRANSACTION time.
    ///
    /// Neither axis alone identifies the set: `occurred_at` cannot say what was
    /// known, and `recorded_seq` cannot say what the record was about.
    pub fn valid_in_window_known_by(
        &self,
        from: &str,
        to: &str,
        known_by_seq: i64,
        limit: usize,
    ) -> Result<Vec<TimelineEntry>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT recorded_seq, record_digest, claim, evidence_class,
                    recorded_at, occurred_at, superseded_seq, superseded_by
             FROM memories
             WHERE occurred_at IS NOT NULL
               AND occurred_at >= ?1 AND occurred_at <= ?2
               AND recorded_seq <= ?3
             ORDER BY occurred_at, recorded_seq
             LIMIT ?4",
        )?;
        let rows = stmt.query_map(params![from, to, known_by_seq, limit as i64], |r| {
            Ok(TimelineEntry {
                recorded_seq: r.get(0)?,
                record_digest: r.get(1)?,
                claim: r.get(2)?,
                evidence_class: r.get(3)?,
                recorded_at: r.get(4)?,
                occurred_at: r.get(5)?,
                superseded_seq: r.get(6)?,
                superseded_by: r.get(7)?,
            })
        })?;
        let mut out = Vec::new();
        for x in rows {
            out.push(x?);
        }
        Ok(out)
    }
}
