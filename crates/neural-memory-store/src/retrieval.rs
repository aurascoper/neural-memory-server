//! Lexical retrieval, provenance traversal, and the merge between them.
//!
//! Shape copied from `claude-mind-mcp`'s `RecallService.swift`: independent
//! branch queries merged **application-side** with per-hit attribution, not a
//! SQL `UNION`. (That was verified against the source — its three branches run
//! concurrently and are merged in Swift by a UUID-keyed accumulator; the SQL
//! contains no `UNION` at all.) M1 has no vector branch, so there are two.
//!
//! ## The rerank weights are re-derived, not renormalised
//!
//! `claude-mind-mcp` uses semantic 0.55, recency 0.20, graph 0.10, lexical 0.15.
//! M1 has no semantic term. The tempting move — drop 0.55 and rescale the
//! remaining 0.45 to sum to 1 — would silently take recency from 0.20 to
//! **0.44**, making it nearly three times as important as lexical relevance.
//! In a store whose entire content is dated measurements, that is close to the
//! worst possible ranking: a benchmark from last month is not less true than one
//! from today.
//!
//! Derived from what this store actually holds:
//!
//! | signal | weight | why |
//! |---|---:|---|
//! | lexical  | 0.70 | the only topical signal M1 has |
//! | graph    | 0.25 | a record reachable from a hit is corroborating evidence |
//! | recency  | 0.05 | **tiebreaker only** — measurements do not decay, and retirement is explicit via supersession, never implied by age |
//!
//! The invariant that matters is pinned by test: recency must never be able to
//! flip a ranking that lexical relevance decides.
//!
//! ## Time
//!
//! Recency is computed against a caller-supplied `as_of`, never `now()`. Two
//! reasons: retrieval must be reproducible for the same inputs, and a store that
//! reads the clock has quietly acquired the dependency the mobile core's
//! doctrine forbids. Age arithmetic uses SQLite's `julianday`, so no date crate
//! is needed anywhere.

use std::collections::BTreeMap;

use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::{Store, StoreError};

// M1 (no semantic term): lexical 0.70, graph 0.25, recency 0.05.
//
// Re-derived again for M2 rather than renormalised -- the same discipline
// applied when the semantic term was absent. Semantic earns a real share
// because finding a paraphrase the wording missed is the entire reason to add
// it. It does NOT take the largest share, because this store is full of exact
// tokens that carry the meaning: model names, metric names, thread counts,
// digests. "Qwen3 8B at 24 threads" is a lexical question, and an embedding
// that decides 8 and 24 are interchangeable numbers is worse than useless on it.
//
// Recency stays a tiebreaker at 0.05 for the M1 reason, unchanged: measurements
// do not decay, and retirement is explicit rather than implied by age.
pub const W_LEXICAL: f64 = 0.45;
pub const W_SEMANTIC: f64 = 0.30;
pub const W_GRAPH: f64 = 0.20;
pub const W_RECENCY: f64 = 0.05;

/// Long on purpose. A measurement from six months ago is still a measurement;
/// this only breaks ties between otherwise-equal candidates.
pub const RECENCY_HALF_LIFE_DAYS: f64 = 180.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Branch {
    Lexical,
    Semantic,
    Provenance,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// Follow edges src -> dst: "what does this record rest on?"
    Forward,
    /// Follow edges dst -> src: "what rests on this record?"
    Backward,
    Both,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hit {
    pub record_digest: String,
    pub claim: String,
    pub evidence_class: String,
    pub score: f64,
    /// Which branches surfaced this record. Reported so a caller can tell a
    /// direct textual match from something reached only by traversal.
    pub branches: Vec<Branch>,
    pub lexical_score: Option<f64>,
    /// Cosine similarity in the query's embedding space. `None` when this
    /// record carries no vector there, which is different from scoring zero.
    pub semantic_score: Option<f64>,
    pub graph_distance: Option<u32>,
    pub recency_score: f64,
    /// True when this record has been retired. Present only when the caller
    /// explicitly asked for retired records.
    pub superseded: bool,
    pub superseded_by: Option<String>,
    /// Records this one is in explicit conflict with, via a `contradicts` edge
    /// in either direction. **SORTED.**
    ///
    /// Pushed onto every hit rather than left to be discovered through
    /// `get_record`. H6 arm (c) found the asymmetry that made this necessary:
    /// retirement was always pushed to the caller, contradiction had to be
    /// pulled, and nothing told the caller to pull. An agent that only called
    /// `recall` could not see that two claims disagreed, however carefully it
    /// read — and both models tested duly failed to notice.
    pub conflicts_with: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchCounts {
    pub lexical: usize,
    pub semantic: usize,
    pub provenance: usize,
    pub unique: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecallResult {
    pub hits: Vec<Hit>,
    pub counts: BranchCounts,
    /// Retired records that matched and were withheld. Reported rather than
    /// silently dropped, so "there is nothing" is distinguishable from "there is
    /// something and it has been retired".
    pub withheld_retired: Vec<String>,
    /// Conflicting pairs where **both** sides are present in these hits.
    ///
    /// The strongest form of the signal: the caller is holding two records that
    /// contradict each other, and neither is retired, so nothing in the store
    /// says which to believe. Surfacing this at the top level means a caller has
    /// to actively ignore it rather than merely fail to look for it.
    pub conflicting_pairs: Vec<(String, String)>,
}

/// A query vector and the space it lives in. Both are required together: a
/// vector without its space identity cannot be compared to anything safely.
pub struct SemanticQuery<'a> {
    pub profile_identity: &'a str,
    pub vector: &'a [f32],
}

pub struct RecallOptions<'a> {
    pub query: &'a str,
    /// `None` disables the semantic branch entirely, which is the correct
    /// behaviour when no embedder is reachable: lexical and provenance still
    /// work, and the caller is told the branch was absent rather than being
    /// given silently worse results.
    pub semantic: Option<SemanticQuery<'a>>,
    /// Reference instant for recency. Required — the store never reads a clock.
    pub as_of: &'a str,
    pub limit: usize,
    /// Provenance hops from each lexical seed. 0 disables the graph branch.
    pub max_hops: u32,
    /// Default false. Retired records are excluded from ordinary recall and
    /// still retrievable when explicitly asked for.
    pub include_retired: bool,
}

impl Default for RecallOptions<'_> {
    fn default() -> Self {
        Self {
            query: "",
            semantic: None,
            as_of: "2026-01-01T00:00:00Z",
            limit: 20,
            max_hops: 1,
            include_retired: false,
        }
    }
}

/// Turn free text into an FTS5 expression that cannot be a syntax error.
///
/// FTS5 treats `-`, `*`, `:`, `^`, `"` and `NEAR` as operators, so a raw user
/// string is both a syntax hazard and an injection surface into the match
/// grammar. Every token is quoted and OR-joined; a quote inside a token is
/// doubled, which is FTS5's own escape.
fn fts_query(raw: &str) -> String {
    let tokens: Vec<String> = raw
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
        .collect();
    tokens.join(" OR ")
}

fn recency_score(age_days: Option<f64>) -> f64 {
    match age_days {
        // No occurred_at is not "infinitely old"; it is unknown. Treat it as
        // neutral rather than penalising a record for a missing field.
        None => 0.5,
        Some(d) if d <= 0.0 => 1.0,
        Some(d) => 0.5f64.powf(d / RECENCY_HALF_LIFE_DAYS),
    }
}

struct Candidate {
    claim: String,
    evidence_class: String,
    lexical_raw: Option<f64>,
    semantic_raw: Option<f64>,
    graph_distance: Option<u32>,
    age_days: Option<f64>,
    superseded_at: Option<String>,
    superseded_by: Option<String>,
    branches: Vec<Branch>,
}

impl Store {
    /// Hybrid recall: lexical seeds, provenance expansion, application-side merge.
    pub fn recall(&self, opt: &RecallOptions<'_>) -> Result<RecallResult, StoreError> {
        let mut cands: BTreeMap<String, Candidate> = BTreeMap::new();
        let mut withheld = Vec::new();
        let expr = fts_query(opt.query);

        // ---- branch 1: lexical -------------------------------------------
        let mut lexical_seeds: Vec<String> = Vec::new();
        if !expr.is_empty() {
            let mut stmt = self.conn.prepare(
                "SELECT m.record_digest, m.claim, m.evidence_class,
                        bm25(memories_fts, 1.0, 0.5) AS rank,
                        julianday(?2) - julianday(m.occurred_at) AS age_days,
                        m.superseded_at, m.superseded_by, m.retracted_at
                 FROM memories_fts
                 JOIN memories m ON m.recorded_seq = memories_fts.rowid
                 WHERE memories_fts MATCH ?1
                 ORDER BY rank
                 LIMIT ?3",
            )?;
            // Over-fetch so retired rows filtered below do not shrink the result.
            let rows = stmt.query_map(params![expr, opt.as_of, (opt.limit * 4) as i64], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, f64>(3)?,
                    r.get::<_, Option<f64>>(4)?,
                    r.get::<_, Option<String>>(5)?,
                    r.get::<_, Option<String>>(6)?,
                    r.get::<_, Option<String>>(7)?,
                ))
            })?;

            for row in rows {
                let (digest, claim, class, rank, age, sup_at, sup_by, retracted) = row?;
                let retired = sup_at.is_some() || retracted.is_some();
                if retired && !opt.include_retired {
                    withheld.push(digest);
                    continue;
                }
                lexical_seeds.push(digest.clone());
                cands.insert(
                    digest,
                    Candidate {
                        claim,
                        evidence_class: class,
                        // bm25 is negative and more-negative is better; flip it
                        // so larger means more relevant, then normalise below.
                        lexical_raw: Some(-rank),
                        semantic_raw: None,
                        graph_distance: None,
                        age_days: age,
                        superseded_at: sup_at,
                        superseded_by: sup_by,
                        branches: vec![Branch::Lexical],
                    },
                );
            }
        }

        // ---- branch 2: semantic --------------------------------------------
        // Runs before provenance so its hits also seed graph expansion: a
        // record found only by meaning is as good a starting point as one found
        // by wording, and treating it as second-class would waste the branch.
        if let Some(sem) = &opt.semantic {
            let found = self
                .vector_search(
                    sem.profile_identity,
                    sem.vector,
                    opt.limit * 4,
                    opt.include_retired,
                )
                .map_err(|e| StoreError::Migration(e.to_string()))?;
            for h in found {
                let sim = f64::from(h.similarity);
                if let Some(existing) = cands.get_mut(&h.record_digest) {
                    if !existing.branches.contains(&Branch::Semantic) {
                        existing.branches.push(Branch::Semantic);
                    }
                    existing.semantic_raw = Some(sim);
                    continue;
                }
                let Some(c) = self.load_candidate(&h.record_digest, opt.as_of)? else {
                    continue;
                };
                if c.superseded_at.is_some() && !opt.include_retired {
                    withheld.push(h.record_digest);
                    continue;
                }
                lexical_seeds.push(h.record_digest.clone());
                cands.insert(
                    h.record_digest,
                    Candidate {
                        semantic_raw: Some(sim),
                        branches: vec![Branch::Semantic],
                        ..c
                    },
                );
            }
        }

        // ---- branch 3: provenance expansion from the seeds ------------------
        if opt.max_hops > 0 {
            for seed in &lexical_seeds {
                for (digest, depth) in self.traverse(seed, Direction::Both, opt.max_hops)? {
                    if &digest == seed {
                        continue;
                    }
                    if let Some(existing) = cands.get_mut(&digest) {
                        // Surfaced by both branches; record that, and keep the
                        // shortest distance found.
                        if !existing.branches.contains(&Branch::Provenance) {
                            existing.branches.push(Branch::Provenance);
                        }
                        existing.graph_distance =
                            Some(existing.graph_distance.unwrap_or(u32::MAX).min(depth));
                        continue;
                    }
                    let Some(c) = self.load_candidate(&digest, opt.as_of)? else {
                        continue;
                    };
                    let retired = c.superseded_at.is_some();
                    if retired && !opt.include_retired {
                        withheld.push(digest);
                        continue;
                    }
                    cands.insert(
                        digest,
                        Candidate {
                            graph_distance: Some(depth),
                            branches: vec![Branch::Provenance],
                            ..c
                        },
                    );
                }
            }
        }

        // ---- merge and rank ----------------------------------------------
        // Normalise lexical scores across the candidate set: bm25 is unbounded,
        // so an absolute threshold would mean nothing.
        let best = cands
            .values()
            .filter_map(|c| c.lexical_raw)
            .fold(f64::NEG_INFINITY, f64::max);

        let mut hits: Vec<Hit> = cands
            .into_iter()
            .map(|(digest, c)| {
                let lex = c.lexical_raw.map(|v| {
                    if best > 0.0 {
                        (v / best).clamp(0.0, 1.0)
                    } else {
                        1.0
                    }
                });
                // 1 hop -> 1.0, 2 -> 0.5, 3 -> 0.33 ...
                let graph = c
                    .graph_distance
                    .map(|d| 1.0 / (d.max(1) as f64))
                    .unwrap_or(0.0);
                let rec = recency_score(c.age_days);
                // Cosine runs [-1, 1]; a negative similarity is evidence of
                // unrelatedness, not of relevance, so it is floored rather than
                // allowed to drag a record below one carrying no vector at all.
                let sem = c.semantic_raw.map(|v| v.clamp(0.0, 1.0));
                let score = W_LEXICAL * lex.unwrap_or(0.0)
                    + W_SEMANTIC * sem.unwrap_or(0.0)
                    + W_GRAPH * graph
                    + W_RECENCY * rec;
                Hit {
                    record_digest: digest,
                    claim: c.claim,
                    evidence_class: c.evidence_class,
                    score,
                    branches: c.branches,
                    lexical_score: lex,
                    semantic_score: sem,
                    graph_distance: c.graph_distance,
                    recency_score: rec,
                    superseded: c.superseded_at.is_some(),
                    superseded_by: c.superseded_by,
                    conflicts_with: Vec::new(), // filled below, once the set is known
                }
            })
            .collect();

        // Conflict edges, pushed onto the hits. One query over the whole edge
        // table: at this corpus size (hundreds of records) a scan is
        // sub-millisecond, and per-hit queries would buy nothing.
        let mut conflicts: Vec<(String, String)> = Vec::new();
        let mut both_present: Vec<(String, String)> = Vec::new();
        {
            let mut stmt = self.conn.prepare(
                "SELECT src_digest, dst_digest FROM provenance_edges
                 WHERE edge_kind = 'contradicts'",
            )?;
            let rows =
                stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            for row in rows {
                conflicts.push(row?);
            }
        }
        if !conflicts.is_empty() {
            let present: std::collections::BTreeSet<String> =
                hits.iter().map(|h| h.record_digest.clone()).collect();
            for h in hits.iter_mut() {
                let mut with: Vec<String> = conflicts
                    .iter()
                    .filter_map(|(a, b)| {
                        // Symmetric: a contradiction holds in both directions
                        // regardless of which side the edge was written from.
                        if *a == h.record_digest {
                            Some(b.clone())
                        } else if *b == h.record_digest {
                            Some(a.clone())
                        } else {
                            None
                        }
                    })
                    .collect();
                with.sort();
                with.dedup();
                h.conflicts_with = with;
            }
            for (a, b) in &conflicts {
                if present.contains(a) && present.contains(b) {
                    let pair = if a < b {
                        (a.clone(), b.clone())
                    } else {
                        (b.clone(), a.clone())
                    };
                    if !both_present.contains(&pair) {
                        both_present.push(pair);
                    }
                }
            }
        }

        // Ties break on digest, so the order is total and reproducible rather
        // than dependent on map iteration.
        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.record_digest.cmp(&b.record_digest))
        });

        let counts = BranchCounts {
            lexical: hits
                .iter()
                .filter(|h| h.branches.contains(&Branch::Lexical))
                .count(),
            semantic: hits
                .iter()
                .filter(|h| h.branches.contains(&Branch::Semantic))
                .count(),
            provenance: hits
                .iter()
                .filter(|h| h.branches.contains(&Branch::Provenance))
                .count(),
            unique: hits.len(),
        };
        hits.truncate(opt.limit);
        withheld.sort();
        withheld.dedup();

        both_present.sort();
        Ok(RecallResult {
            hits,
            counts,
            withheld_retired: withheld,
            conflicting_pairs: both_present,
        })
    }

    fn load_candidate(&self, digest: &str, as_of: &str) -> Result<Option<Candidate>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT claim, evidence_class,
                    julianday(?2) - julianday(occurred_at),
                    superseded_at, superseded_by
             FROM memories WHERE record_digest = ?1",
        )?;
        let mut rows = stmt.query(params![digest, as_of])?;
        Ok(match rows.next()? {
            None => None,
            Some(r) => Some(Candidate {
                claim: r.get(0)?,
                evidence_class: r.get(1)?,
                lexical_raw: None,
                semantic_raw: None,
                graph_distance: None,
                age_days: r.get(2)?,
                superseded_at: r.get(3)?,
                superseded_by: r.get(4)?,
                branches: Vec::new(),
            }),
        })
    }

    /// Walk the provenance graph from `seed`, returning `(digest, hops)`.
    ///
    /// Cycle-guarded by carrying the visited path: `supersedes` chains and
    /// `derivedFrom` links can readily form a cycle, and a recursive CTE without
    /// a guard does not merely return duplicates — it does not terminate.
    pub fn traverse(
        &self,
        seed: &str,
        direction: Direction,
        max_hops: u32,
    ) -> Result<Vec<(String, u32)>, StoreError> {
        // Expressed as one join with a direction-dependent condition and "next
        // node" expression. SQLite has no `AS alias(col)` subquery column
        // renaming (that is a Postgres extension), and a correlated subquery in
        // a JOIN is fragile besides.
        let (join_cond, next) = match direction {
            Direction::Forward => ("e.src_digest = w.digest", "e.dst_digest"),
            Direction::Backward => ("e.dst_digest = w.digest", "e.src_digest"),
            Direction::Both => (
                "(e.src_digest = w.digest OR e.dst_digest = w.digest)",
                "CASE WHEN e.src_digest = w.digest THEN e.dst_digest ELSE e.src_digest END",
            ),
        };
        let sql = format!(
            "WITH RECURSIVE walk(digest, depth, path) AS (
               SELECT ?1, 0, '/' || ?1 || '/'
               UNION ALL
               SELECT {next}, w.depth + 1, w.path || {next} || '/'
               FROM walk w
               JOIN provenance_edges e ON {join_cond}
               WHERE w.depth < ?2
                 AND instr(w.path, '/' || {next} || '/') = 0
             )
             SELECT digest, min(depth) FROM walk GROUP BY digest ORDER BY min(depth), digest"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![seed, max_hops], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u32))
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Full provenance trace for one record, as an MCP tool would return it.
    pub fn trace_provenance(
        &self,
        digest: &str,
        max_hops: u32,
    ) -> Result<Vec<ProvenanceStep>, StoreError> {
        let mut stmt = self.conn.prepare(
            "WITH RECURSIVE walk(digest, depth, path) AS (
               SELECT ?1, 0, '/' || ?1 || '/'
               UNION ALL
               SELECT e.dst_digest, w.depth + 1, w.path || e.dst_digest || '/'
               FROM walk w JOIN provenance_edges e ON e.src_digest = w.digest
               WHERE w.depth < ?2 AND instr(w.path, '/' || e.dst_digest || '/') = 0
             )
             SELECT w.digest, w.depth, m.claim, m.evidence_class,
                    (SELECT group_concat(edge_kind) FROM provenance_edges
                     WHERE dst_digest = w.digest)
             FROM walk w LEFT JOIN memories m ON m.record_digest = w.digest
             WHERE w.depth > 0
             GROUP BY w.digest
             ORDER BY min(w.depth), w.digest",
        )?;
        let rows = stmt.query_map(params![digest, max_hops], |r| {
            Ok(ProvenanceStep {
                record_digest: r.get(0)?,
                hops: r.get::<_, i64>(1)? as u32,
                claim: r.get(2)?,
                evidence_class: r.get(3)?,
                edge_kinds: r
                    .get::<_, Option<String>>(4)?
                    .map(|s| s.split(',').map(str::to_string).collect())
                    .unwrap_or_default(),
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProvenanceStep {
    pub record_digest: String,
    pub hops: u32,
    /// `None` when an edge points at something that is not a stored claim — an
    /// artifact, say. Surfaced rather than hidden by an inner join.
    pub claim: Option<String>,
    pub evidence_class: Option<String>,
    pub edge_kinds: Vec<String>,
}

/// Everything about one record: the claim, what it rests on, and its edges.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordDetail {
    pub record_digest: String,
    pub claim: String,
    pub evidence_class: String,
    pub recorded_seq: i64,
    pub occurred_at: Option<String>,
    pub source_artifact_sha256: Option<String>,
    pub source_locator: Option<String>,
    pub superseded_by: Option<String>,
    pub superseded_at: Option<String>,
    pub retracted_at: Option<String>,
    pub retraction_reason: Option<String>,
    /// Observations this claim rests on, each with its measurement policy and
    /// reference execution resolved -- so a cited number always arrives with
    /// what it was measured by and against.
    pub observations: Vec<ObservationDetail>,
    pub edges: Vec<EdgeDetail>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservationDetail {
    pub identity: String,
    pub observation_kind: String,
    pub quantity_kind: String,
    pub value_text: String,
    pub metric: String,
    pub aggregation: String,
    pub unit: String,
    pub step_budget: Option<u32>,
    pub evaluation_suite: String,
    /// `None` only for absolute quantities; a relative one cannot be stored
    /// without it.
    pub reference_execution: Option<String>,
    pub reference_backend: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EdgeDetail {
    pub direction: String,
    pub edge_kind: String,
    pub other_digest: String,
}

impl Store {
    pub fn get_record(&self, digest: &str) -> Result<Option<RecordDetail>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT record_digest, claim, evidence_class, recorded_seq, occurred_at,
                    source_artifact_sha256, source_locator, superseded_by, superseded_at,
                    retracted_at, retraction_reason
             FROM memories WHERE record_digest = ?1",
        )?;
        let mut rows = stmt.query(params![digest])?;
        let Some(r) = rows.next()? else {
            return Ok(None);
        };
        let mut detail = RecordDetail {
            record_digest: r.get(0)?,
            claim: r.get(1)?,
            evidence_class: r.get(2)?,
            recorded_seq: r.get(3)?,
            occurred_at: r.get(4)?,
            source_artifact_sha256: r.get(5)?,
            source_locator: r.get(6)?,
            superseded_by: r.get(7)?,
            superseded_at: r.get(8)?,
            retracted_at: r.get(9)?,
            retraction_reason: r.get(10)?,
            observations: Vec::new(),
            edges: Vec::new(),
        };
        drop(rows);

        let mut o = self.conn.prepare(
            "SELECT o.identity, o.observation_kind, o.quantity_kind, o.value_text,
                    p.metric, p.aggregation, p.unit, p.step_budget,
                    s.suite_name, o.reference_execution_identity, r.backend_id
             FROM memory_observations mo
             JOIN observations o ON o.identity = mo.observation_identity
             JOIN measurement_policies p ON p.identity = o.measurement_policy_identity
             JOIN evaluation_suites s ON s.identity = o.evaluation_suite_identity
             LEFT JOIN reference_executions r ON r.identity = o.reference_execution_identity
             WHERE mo.record_digest = ?1
             ORDER BY o.identity",
        )?;
        let obs = o.query_map(params![digest], |r| {
            Ok(ObservationDetail {
                identity: r.get(0)?,
                observation_kind: r.get(1)?,
                quantity_kind: r.get(2)?,
                value_text: r.get(3)?,
                metric: r.get(4)?,
                aggregation: r.get(5)?,
                unit: r.get(6)?,
                step_budget: r.get::<_, Option<i64>>(7)?.map(|v| v as u32),
                evaluation_suite: r.get(8)?,
                reference_execution: r.get(9)?,
                reference_backend: r.get(10)?,
            })
        })?;
        for x in obs {
            detail.observations.push(x?);
        }

        let mut e = self.conn.prepare(
            "SELECT 'outgoing', edge_kind, dst_digest FROM provenance_edges WHERE src_digest = ?1
             UNION ALL
             SELECT 'incoming', edge_kind, src_digest FROM provenance_edges WHERE dst_digest = ?1
             ORDER BY 1, 2, 3",
        )?;
        let edges = e.query_map(params![digest], |r| {
            Ok(EdgeDetail {
                direction: r.get(0)?,
                edge_kind: r.get(1)?,
                other_digest: r.get(2)?,
            })
        })?;
        for x in edges {
            detail.edges.push(x?);
        }
        Ok(Some(detail))
    }
}

/// An unresolved disagreement touching evidence the caller has cited.
///
/// "Unresolved" is the operative word: if one side is retired, the store knows
/// which claim is current and there is nothing for the caller to adjudicate.
/// An obligation is raised only where the store genuinely cannot tell.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictObligation {
    /// The cited record.
    pub cited: String,
    pub cited_claim: String,
    /// What it conflicts with.
    pub conflicts_with: String,
    pub conflicting_claim: String,
}

impl Store {
    /// Conflicts the caller must account for, given what it chose to cite.
    ///
    /// Computed from the cited set alone, so it needs no session state: if you
    /// rest an answer on a record, you inherit that record's disagreements
    /// whether or not you noticed them.
    pub fn conflict_obligations(
        &self,
        cited: &[String],
    ) -> Result<Vec<ConflictObligation>, StoreError> {
        let mut out = Vec::new();
        let mut stmt = self.conn.prepare(
            "SELECT other.record_digest, other.claim, me.claim
             FROM provenance_edges e
             JOIN memories me    ON me.record_digest = ?1
             JOIN memories other ON other.record_digest =
                 CASE WHEN e.src_digest = ?1 THEN e.dst_digest ELSE e.src_digest END
             WHERE e.edge_kind = 'contradicts'
               AND (e.src_digest = ?1 OR e.dst_digest = ?1)
               -- A retired counterpart is not an obligation: the store already
               -- says which is current, so there is nothing to adjudicate.
               AND other.superseded_at IS NULL
               AND me.superseded_at IS NULL",
        )?;
        for c in cited {
            let rows = stmt.query_map(params![c], |r| {
                Ok(ConflictObligation {
                    cited: c.clone(),
                    cited_claim: r.get(2)?,
                    conflicts_with: r.get(0)?,
                    conflicting_claim: r.get(1)?,
                })
            })?;
            for row in rows {
                out.push(row?);
            }
        }
        out.sort_by(|a, b| (&a.cited, &a.conflicts_with).cmp(&(&b.cited, &b.conflicts_with)));
        out.dedup_by(|a, b| a.cited == b.cited && a.conflicts_with == b.conflicts_with);
        Ok(out)
    }
}
