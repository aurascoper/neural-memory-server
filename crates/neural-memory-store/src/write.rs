//! The append-only write path.
//!
//! Two properties this module exists to guarantee:
//!
//! **Idempotency.** Every write is keyed by its content digest. Re-running an
//! import produces no duplicate rows, no new `recorded_seq` values, and no
//! spurious history. Callers are told which happened rather than left to guess.
//!
//! **Evidence class is derived, never accepted.** This applies ADR-002 A3's
//! doctrine — *"hashing the caller's own label into its digest made the identity
//! self-referential"* — to trust rather than to identity. A writer that can
//! declare its own trustworthiness has not been constrained.

use neural_memory_domain::{
    artifact_identity, evaluation_suite_identity, measurement_policy_identity,
    memory_record_identity, observation_identity, reference_execution_identity,
    validate_observation, ArtifactTerms, EvaluationSuiteTerms, EvidenceClass,
    MeasurementPolicyTerms, MemoryRecordTerms, ObservationDefect, ObservationTerms, QuantityKind,
    ReferenceExecutionTerms,
};
use rusqlite::{params, OptionalExtension};

use crate::derive::{Derivation, DerivationError};

/// Mirror of the bound in `0004_standing.sql`. The trigger is authoritative —
/// this exists so the refusal can be informative, and
/// `the_rust_budget_matches_the_trigger` fails if they drift apart.
pub const STANDING_BUDGET_CHARS: usize = 6000;
use crate::{Store, StoreError};

/// Which door a write came through.
///
/// This is not advisory. `Agent` writes are clamped to `AgentInference` *before*
/// the record digest is computed, so an agent cannot even produce the bytes of a
/// higher-trust record — the digest would be a different record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteChannel {
    /// The MCP surface, reachable by a model.
    Agent,
    /// An out-of-band importer or operator CLI, which no agent tool can invoke.
    Operator,
}

/// Did this write create something, or was it already there?
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wrote {
    Inserted,
    AlreadyPresent,
}

impl Wrote {
    pub fn inserted(self) -> bool {
        self == Wrote::Inserted
    }
}

#[derive(Debug)]
pub enum WriteError {
    Store(StoreError),
    Sql(rusqlite::Error),
    Observation(Vec<ObservationDefect>),
    Evidence(EvidenceRefusal),
    Standing(StandingRefusal),
}

/// Why a record was refused the standing disposition.
///
/// Separate from `EvidenceRefusal` because these are not claims about how
/// something came to be believed -- they are claims about who may make a record
/// bind, and what it costs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StandingRefusal {
    /// The promotion itself must be a human act.
    ///
    /// The database trigger checks `evidence_class = 'humanDecision'`, which
    /// stops an agent promoting a record it MINTED. It does not stop an agent
    /// promoting a record a HUMAN minted for some other purpose -- that would
    /// bypass the guard at the promotion step rather than the minting step, and
    /// the result is the same: text an agent chose, in front of every future
    /// session. Minting and promoting are two acts and both must be human.
    OperatorOnly,
    NoSuchRecord {
        digest: String,
    },
    /// Only a decision may bind. A measurement is not a rule and arithmetic is
    /// not a rule.
    OnlyDecisionsCanBind {
        evidence_class: String,
    },
    /// Declaring a retired record standing would put a withdrawn rule in force.
    AlreadyRetired {
        digest: String,
    },
    /// Context is not free, and the refusal says what is already competing --
    /// otherwise choosing what to retire needs a separate query at the one
    /// moment someone must choose.
    WouldExceedBudget {
        current: Vec<(String, usize)>,
        adding: usize,
        limit: usize,
    },
}

impl std::fmt::Display for StandingRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StandingRefusal::OperatorOnly => write!(
                f,
                "declaring a record standing is an operator action: it puts text \
                 in front of every future session before any work begins"
            ),
            StandingRefusal::NoSuchRecord { digest } => {
                write!(f, "no record {digest}")
            }
            StandingRefusal::OnlyDecisionsCanBind { evidence_class } => write!(
                f,
                "only a humanDecision may stand, this record is {evidence_class}: \
                 a measurement is not a rule and arithmetic is not a rule"
            ),
            StandingRefusal::AlreadyRetired { digest } => {
                write!(f, "record {digest} is retired; a withdrawn rule may not bind")
            }
            StandingRefusal::WouldExceedBudget {
                current,
                adding,
                limit,
            } => {
                let used: usize = current.iter().map(|(_, n)| n).sum();
                writeln!(
                    f,
                    "standing set would exceed its budget: {used} + {adding} > {limit} chars.\n\
                     Retire one of these before adding another:"
                )?;
                for (digest, n) in current {
                    writeln!(f, "  {:>5}  {}", n, &digest[..16.min(digest.len())])?;
                }
                write!(f, "  {used:>5}  total")
            }
        }
    }
}

/// Why a claimed evidence class was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EvidenceRefusal {
    /// `Observed` means an artifact exists and was ingested out of band. A claim
    /// with no artifact is an inference about the world, not an observation of it.
    ObservedNeedsArtifact,
    ObservedArtifactUnknown {
        sha256: String,
    },
    /// `DerivedDeterministically` without a transform cannot be recomputed, so
    /// it cannot be distinguished from an assertion.
    DerivedNeedsDerivation,
    /// The transform was named, the inputs resolved, and the arithmetic did not
    /// agree with the claim.
    DerivationFailed(DerivationError),
    // Deliberately no `HumanDecisionNeedsOperator`: the Agent channel clamps to
    // AgentInference before `check_evidence` is ever reached, so such a variant
    // could never be constructed. Upstream's `ContractIdentityNotSealed` is
    // exactly that -- declared, never built, and therefore a claim about
    // enforcement that nothing tests. One dead variant is enough.
}

impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WriteError::Store(e) => write!(f, "{e}"),
            WriteError::Sql(e) => write!(f, "sqlite: {e}"),
            WriteError::Observation(d) => write!(f, "invalid observation: {d:?}"),
            WriteError::Evidence(r) => write!(f, "evidence refused: {r:?}"),
            WriteError::Standing(r) => write!(f, "standing refused: {r}"),
        }
    }
}

impl std::error::Error for WriteError {}

impl From<rusqlite::Error> for WriteError {
    fn from(e: rusqlite::Error) -> Self {
        WriteError::Sql(e)
    }
}
impl From<StoreError> for WriteError {
    fn from(e: StoreError) -> Self {
        WriteError::Store(e)
    }
}

/// A memory record plus whatever justifies its evidence class.
pub struct MemoryWrite<'a> {
    pub terms: MemoryRecordTerms,
    /// Valid time: when the claim was true of the world.
    pub occurred_at: Option<&'a str>,
    /// Transaction time: when the store came to believe it. Supplied by the
    /// caller rather than read from a clock, so an import is reproducible and
    /// the two axes cannot silently collapse into one.
    pub recorded_at: Option<&'a str>,
    /// Required when claiming `DerivedDeterministically`.
    pub derivation: Option<Derivation>,
}

impl Store {
    // -- reference data -----------------------------------------------------

    pub fn put_artifact(
        &self,
        a: &ArtifactTerms,
        ingested_at: &str,
    ) -> Result<(String, Wrote), WriteError> {
        let identity = artifact_identity(a);
        let n = self.conn.execute(
            "INSERT OR IGNORE INTO artifacts
               (sha256_hex, artifact_kind, byte_size, media_type, source_uri, ingested_at)
             VALUES (?1,?2,?3,?4,?5,?6)",
            params![
                a.sha256_hex,
                a.artifact_kind,
                a.byte_size as i64,
                a.media_type,
                a.source_uri,
                ingested_at
            ],
        )?;
        Ok((identity, wrote(n)))
    }

    pub fn put_measurement_policy(
        &self,
        p: &MeasurementPolicyTerms,
    ) -> Result<(String, Wrote), WriteError> {
        let identity = measurement_policy_identity(p);
        let n = self.conn.execute(
            "INSERT OR IGNORE INTO measurement_policies
               (identity, metric, aggregation, comparison_rule, step_budget, unit)
             VALUES (?1,?2,?3,?4,?5,?6)",
            params![
                identity,
                p.metric,
                p.aggregation,
                p.comparison_rule,
                p.step_budget,
                p.unit
            ],
        )?;
        Ok((identity, wrote(n)))
    }

    pub fn put_evaluation_suite(
        &self,
        s: &EvaluationSuiteTerms,
    ) -> Result<(String, Wrote), WriteError> {
        let identity = evaluation_suite_identity(s);
        let mut cases = s.case_digests.clone();
        cases.sort();
        let n = self.conn.execute(
            "INSERT OR IGNORE INTO evaluation_suites
               (identity, suite_name, case_digests, tokenizer_identity, context_cap)
             VALUES (?1,?2,?3,?4,?5)",
            params![
                identity,
                s.suite_name,
                serde_json::to_string(&cases).expect("json"),
                s.tokenizer_identity,
                s.context_cap
            ],
        )?;
        Ok((identity, wrote(n)))
    }

    pub fn put_reference_execution(
        &self,
        r: &ReferenceExecutionTerms,
    ) -> Result<(String, Wrote), WriteError> {
        let identity = reference_execution_identity(r);
        let mut env = r.environment.clone();
        env.sort();
        let n = self.conn.execute(
            "INSERT OR IGNORE INTO reference_executions
               (identity, runtime_identity, backend_id, artifact_sha256,
                evaluation_suite_identity, environment)
             VALUES (?1,?2,?3,?4,?5,?6)",
            params![
                identity,
                r.runtime_identity,
                r.backend_id,
                r.artifact_sha256,
                r.evaluation_suite_identity,
                serde_json::to_string(&env).expect("json")
            ],
        )?;
        Ok((identity, wrote(n)))
    }

    // -- observations -------------------------------------------------------

    pub fn put_observation(
        &self,
        o: &ObservationTerms,
        observed_at: &str,
    ) -> Result<(String, Wrote), WriteError> {
        // Validated in the pure layer before the SQL CHECK also sees it. Both
        // layers enforce the referent rule deliberately: a rule that lives in
        // one place can be bypassed by writing through the other.
        let defects = validate_observation(o);
        if !defects.is_empty() {
            return Err(WriteError::Observation(defects));
        }
        let identity = observation_identity(o);
        let kind = match o.quantity_kind {
            QuantityKind::Absolute => "absolute",
            QuantityKind::Relative => "relative",
        };
        let n = self.conn.execute(
            "INSERT OR IGNORE INTO observations
               (identity, observation_kind, quantity_kind, value_text, value_real,
                measurement_policy_identity, evaluation_suite_identity,
                reference_execution_identity, runtime_identity, artifact_sha256, observed_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            params![
                identity,
                o.observation_kind,
                kind,
                o.value_text,
                o.value_text.parse::<f64>().ok(),
                o.measurement_policy_identity,
                o.evaluation_suite_identity,
                o.reference_execution_identity,
                o.runtime_identity,
                o.artifact_sha256,
                observed_at
            ],
        )?;
        Ok((identity, wrote(n)))
    }

    // -- claims -------------------------------------------------------------

    /// Write a claim. `channel` decides how much the record is allowed to assert.
    ///
    /// Returns the record digest and whether it was newly inserted.
    pub fn put_memory(
        &self,
        channel: WriteChannel,
        w: &MemoryWrite<'_>,
    ) -> Result<(String, Wrote), WriteError> {
        let mut terms = w.terms.clone();

        // An agent's claim is an agent's claim. Clamping happens BEFORE the
        // digest is computed, so the higher-trust record is not merely rejected
        // — it is unrepresentable through this door.
        if channel == WriteChannel::Agent {
            terms.evidence_class = EvidenceClass::AgentInference;
        } else {
            self.check_evidence(&terms, w.derivation.as_ref())
                .map_err(WriteError::Evidence)?;
        }

        let digest = memory_record_identity(&terms);
        let class = match terms.evidence_class {
            EvidenceClass::Observed => "observed",
            EvidenceClass::DerivedDeterministically => "derivedDeterministically",
            EvidenceClass::HumanDecision => "humanDecision",
            EvidenceClass::AgentInference => "agentInference",
            EvidenceClass::ExternalClaim => "externalClaim",
        };

        let n = self.conn.execute(
            "INSERT OR IGNORE INTO memories
               (record_digest, claim, evidence_class, source_artifact_sha256,
                source_locator, harness_run_id, occurred_at, recorded_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                digest,
                terms.claim,
                class,
                terms.source_artifact_sha256,
                terms.source_locator,
                terms.harness_run_id,
                w.occurred_at,
                w.recorded_at
            ],
        )?;

        // Link supporting observations. Also idempotent.
        for obs in &terms.observation_identities {
            self.conn.execute(
                "INSERT OR IGNORE INTO memory_observations VALUES (?1,?2)",
                params![digest, obs],
            )?;
        }
        Ok((digest, wrote(n)))
    }

    /// Derive-or-refuse. Never trusts the declared class on an operator write.
    fn check_evidence(
        &self,
        terms: &MemoryRecordTerms,
        derivation: Option<&Derivation>,
    ) -> Result<(), EvidenceRefusal> {
        match terms.evidence_class {
            EvidenceClass::Observed => {
                let sha = terms
                    .source_artifact_sha256
                    .as_deref()
                    .ok_or(EvidenceRefusal::ObservedNeedsArtifact)?;
                let known: Option<i64> = self
                    .conn
                    .query_row(
                        "SELECT 1 FROM artifacts WHERE sha256_hex = ?1",
                        params![sha],
                        |r| r.get(0),
                    )
                    .optional()
                    .map_err(|_| EvidenceRefusal::ObservedArtifactUnknown {
                        sha256: sha.to_string(),
                    })?;
                known.ok_or(EvidenceRefusal::ObservedArtifactUnknown {
                    sha256: sha.to_string(),
                })?;
                Ok(())
            }
            EvidenceClass::DerivedDeterministically => {
                let d = derivation.ok_or(EvidenceRefusal::DerivedNeedsDerivation)?;
                self.verify_derivation(d, terms)
                    .map_err(EvidenceRefusal::DerivationFailed)
            }
            // Reached only on the Operator channel; the Agent branch never gets here.
            EvidenceClass::HumanDecision => Ok(()),
            EvidenceClass::AgentInference | EvidenceClass::ExternalClaim => Ok(()),
        }
    }

    /// Resolve the named inputs and re-run the arithmetic.
    ///
    /// If it cannot be recomputed, it is not a deterministic derivation, and the
    /// write is refused rather than downgraded — a silent downgrade would leave a
    /// claim in the store whose class no longer matched what the writer meant.
    fn verify_derivation(
        &self,
        d: &Derivation,
        terms: &MemoryRecordTerms,
    ) -> Result<(), DerivationError> {
        let mut values = Vec::new();
        for id in d.inputs() {
            let v: Option<String> = self
                .conn
                .query_row(
                    "SELECT value_text FROM observations WHERE identity = ?1",
                    params![id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|_| DerivationError::UnknownInput {
                    identity: id.to_string(),
                })?;
            let text = v.ok_or_else(|| DerivationError::UnknownInput {
                identity: id.to_string(),
            })?;
            let parsed = text
                .parse::<f64>()
                .map_err(|_| DerivationError::InputNotDecimal {
                    identity: id.to_string(),
                    value_text: text.clone(),
                })?;
            values.push(parsed);
        }
        // The claim text carries the derived value; it is the last whitespace-
        // separated decimal token. Keeping the claim human-readable and the
        // check mechanical is the point.
        let claimed = terms
            .claim
            .split_whitespace()
            .rev()
            .find_map(|t| {
                let cleaned: String = t
                    .chars()
                    .filter(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
                    .collect();
                cleaned.parse::<f64>().ok().map(|_| cleaned)
            })
            .ok_or_else(|| DerivationError::ClaimNotDecimal {
                claimed: terms.claim.clone(),
            })?;
        d.verify(&values, &claimed)
    }

    // -- retirement ---------------------------------------------------------

    /// Retire a claim in favour of another.
    ///
    /// The retired record is not deleted and its text is not edited. Default
    /// retrieval will hide it, explicit retrieval still returns it, and any
    /// session prefix that already showed it keeps it — rewriting history would
    /// invalidate the prefix *and* erase that the belief changed.
    pub fn supersede(&self, retired: &str, replacement: &str, at: &str) -> Result<(), WriteError> {
        let tx_conn = &self.conn;
        // The supersession happens NOW in transaction order. The current high
        // watermark is that point: the replacement already exists, so the store
        // could not have known this any earlier.
        let now_seq: i64 = tx_conn.query_row(
            "SELECT coalesce(max(recorded_seq), 0) FROM memories",
            [],
            |r| r.get(0),
        )?;
        tx_conn.execute(
            "UPDATE memories SET superseded_by = ?2, superseded_at = ?3, superseded_seq = ?4
             WHERE record_digest = ?1",
            params![retired, replacement, at, now_seq],
        )?;
        tx_conn.execute(
            "INSERT OR IGNORE INTO provenance_edges VALUES (?1,?2,'supersedes',?3)",
            params![replacement, retired, at],
        )?;
        Ok(())
    }

    /// Declare a record standing: in force, injected before any session works.
    ///
    /// TWO ACTS, BOTH HUMAN. `write.rs` already clamps the agent channel to
    /// `AgentInference` before a digest exists, so an agent cannot MINT a record
    /// eligible to stand, and the database trigger enforces that at the SQL
    /// layer. Neither stops an agent PROMOTING a record a human minted for some
    /// other purpose — that bypasses the guard at the promotion step instead of
    /// the minting step, and the outcome is identical: text an agent chose, in
    /// front of every future session, indefinitely. So the channel is checked
    /// here, first, before anything is read.
    ///
    /// The trigger is the layer that cannot be routed around; this is the layer
    /// that says why, and — for the budget — what is already competing.
    pub fn declare_standing(
        &self,
        channel: WriteChannel,
        digest: &str,
        at: &str,
    ) -> Result<(), WriteError> {
        if channel == WriteChannel::Agent {
            return Err(WriteError::Standing(StandingRefusal::OperatorOnly));
        }

        let row: Option<(String, Option<String>, Option<String>, usize)> = self
            .conn
            .query_row(
                "SELECT evidence_class, superseded_at, retracted_at, length(claim)
                   FROM memories WHERE record_digest = ?1",
                params![digest],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get::<_, i64>(3)? as usize)),
            )
            .optional()?;
        let Some((class, superseded, retracted, claim_len)) = row else {
            return Err(WriteError::Standing(StandingRefusal::NoSuchRecord {
                digest: digest.to_string(),
            }));
        };
        if class != "humanDecision" {
            return Err(WriteError::Standing(StandingRefusal::OnlyDecisionsCanBind {
                evidence_class: class,
            }));
        }
        if superseded.is_some() || retracted.is_some() {
            return Err(WriteError::Standing(StandingRefusal::AlreadyRetired {
                digest: digest.to_string(),
            }));
        }

        // Budget, re-checked here so the refusal can name what is competing.
        // The trigger enforces the same bound and is what makes it unbypassable.
        let mut stmt = self
            .conn
            .prepare("SELECT record_digest, length(claim) FROM standing_current")?;
        let current: Vec<(String, usize)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get::<_, i64>(1)? as usize)))?
            .collect::<Result<_, _>>()?;
        let used: usize = current.iter().map(|(_, n)| n).sum();
        if used + claim_len > STANDING_BUDGET_CHARS {
            return Err(WriteError::Standing(StandingRefusal::WouldExceedBudget {
                current,
                adding: claim_len,
                limit: STANDING_BUDGET_CHARS,
            }));
        }

        // Same watermark reasoning as `supersede` and `retract`: the declaration
        // happens NOW in transaction order.
        let now_seq: i64 = self.conn.query_row(
            "SELECT coalesce(max(recorded_seq), 0) FROM memories",
            [],
            |r| r.get(0),
        )?;
        self.conn.execute(
            "UPDATE memories SET standing_from = ?2, standing_seq = ?3
             WHERE record_digest = ?1",
            params![digest, at, now_seq],
        )?;
        Ok(())
    }

    /// Retract a record: withdraw it with a stated reason and no replacement.
    ///
    /// Distinct from `supersede`, which says "this was replaced by that".
    /// Retraction says "this should not have been believed" and names why.
    /// The schema has carried `retracted_at`/`retracted_seq`/`retraction_reason`
    /// since 0001 and four read sites honour them, but nothing ever wrote them —
    /// so "retirement" meant supersession alone, and the retracted half of every
    /// retirement check was dead code that could not be tested.
    ///
    /// Nothing is deleted or edited: the row stays, default retrieval hides it,
    /// `include_retired` still returns it. The reason is required by the schema
    /// CHECK — a retraction with no stated reason is not a retraction.
    pub fn retract(&self, digest: &str, reason: &str, at: &str) -> Result<(), WriteError> {
        let tx_conn = &self.conn;
        // Same watermark reasoning as `supersede`: the withdrawal happens NOW in
        // transaction order, and the store could not have known it earlier.
        let now_seq: i64 = tx_conn.query_row(
            "SELECT coalesce(max(recorded_seq), 0) FROM memories",
            [],
            |r| r.get(0),
        )?;
        tx_conn.execute(
            "UPDATE memories SET retracted_at = ?2, retracted_seq = ?3, retraction_reason = ?4
             WHERE record_digest = ?1",
            params![digest, at, now_seq, reason],
        )?;
        Ok(())
    }

    pub fn add_edge(
        &self,
        src: &str,
        dst: &str,
        kind: &str,
        at: &str,
    ) -> Result<Wrote, WriteError> {
        let n = self.conn.execute(
            "INSERT OR IGNORE INTO provenance_edges VALUES (?1,?2,?3,?4)",
            params![src, dst, kind, at],
        )?;
        Ok(wrote(n))
    }

    /// Highest `recorded_seq`. Used to prove an import added no history.
    pub fn max_recorded_seq(&self) -> Result<i64, StoreError> {
        Ok(self.conn.query_row(
            "SELECT coalesce(max(recorded_seq), 0) FROM memories",
            [],
            |r| r.get(0),
        )?)
    }
}

fn wrote(rows: usize) -> Wrote {
    if rows == 0 {
        Wrote::AlreadyPresent
    } else {
        Wrote::Inserted
    }
}
