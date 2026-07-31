//! Record terms and their sealed identities.
//!
//! Deterministic and effect-free: no I/O, no clock.
//!
//! ## The seal doctrine, inherited
//!
//! From `neuralcompose-mobile-core/src/conformance.rs:132-138`:
//!
//! > Canonical identity of a numerical contract, derived from the substantive
//! > TERMS and never from the declared label. Hashing the caller-supplied id into
//! > its own digest would make the identity self-referential — an assertion
//! > rather than a seal.
//!
//! Every `*_identity()` here follows that: the identity is computed from terms,
//! and no type carries its own id as an input to its own digest.
//!
//! ## Sort discipline is per-field, not blanket
//!
//! Upstream sorts every list because declaration order there is noise
//! (`model_formats`, `libraries`, `licenses`). That is wrong for fields where
//! order carries meaning — the TP9/AF7/AF8/TP10 channel-order case, or the
//! append order of an assembled context. Every list field below is therefore
//! marked **SORTED** or **ORDER SIGNIFICANT**, and `tests/list_order.rs` asserts
//! both polarities: reordering a SORTED list must not change the digest;
//! reordering an ORDER SIGNIFICANT one must.

use serde::{Deserialize, Serialize};

use crate::digest::{seal, valid_sha256};

const MEASUREMENT_POLICY_DOMAIN: &str = "neuralmemory.measurement-policy.v1";
const EVALUATION_SUITE_DOMAIN: &str = "neuralmemory.evaluation-suite.v1";
const REFERENCE_EXECUTION_DOMAIN: &str = "neuralmemory.reference-execution.v1";
const OBSERVATION_DOMAIN: &str = "neuralmemory.observation.v1";
const ARTIFACT_DOMAIN: &str = "neuralmemory.artifact.v1";
const MEMORY_RECORD_DOMAIN: &str = "neuralmemory.memory-record.v1";

// ---------------------------------------------------------------------------
// Evidence class
// ---------------------------------------------------------------------------

/// How a claim came to be believed.
///
/// This is **derived from verifiable structure, never asserted by the writer** —
/// the same rule ADR-002 A3 applies to contract ids. The MCP write surface is
/// physically incapable of emitting anything above `AgentInference`; leaving that
/// default requires positive proof, checked in the store layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EvidenceClass {
    /// Backed by a content-addressed artifact ingested through a channel no
    /// agent tool can reach, with command line and exit code.
    Observed,
    /// A named transform over named input digests, which the store re-executes
    /// and compares. If it cannot recompute, the write is rejected.
    DerivedDeterministically,
    /// Recorded through an out-of-band CLI the agent has no tool for.
    HumanDecision,
    /// The default. Advisory only.
    AgentInference,
    /// Asserted by a third party; requires verification before governing anything.
    ExternalClaim,
}

// ---------------------------------------------------------------------------
// Measurement policy — this is the C6 gap being closed
// ---------------------------------------------------------------------------

/// *What was measured, how, and against what rule.*
///
/// This type exists because of a defect found in the generation contract:
/// `BackendObservation.max_logit_divergence` is a bare `f64` with **no declared
/// metric**. Cosine? Max absolute element error? L2? RMS? Two labs could both
/// report `Conformant` against the same seal having measured different things.
/// A number without a stated metric has no bound referent.
///
/// Nothing here may be defaulted. If you cannot name the metric, you do not have
/// a measurement — you have a number.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasurementPolicyTerms {
    /// The metric itself, e.g. `maxAbsoluteLogitDelta`, `cosineSimilarity`.
    pub metric: String,
    /// How per-step values collapse to one number, e.g. `maxOverSteps`.
    pub aggregation: String,
    /// How the aggregate is compared, e.g. `lessThanOrEqualTolerance`.
    pub comparison_rule: String,
    /// Steps/cases considered. `None` means unbounded, which is a claim in itself.
    pub step_budget: Option<u32>,
    /// Unit of the resulting quantity, e.g. `logit`, `second`, `token`.
    pub unit: String,
}

pub fn measurement_policy_identity(p: &MeasurementPolicyTerms) -> String {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Doc<'a> {
        domain: &'static str,
        metric: &'a str,
        aggregation: &'a str,
        comparison_rule: &'a str,
        step_budget: Option<u32>,
        unit: &'a str,
    }
    seal(&Doc {
        domain: MEASUREMENT_POLICY_DOMAIN,
        metric: &p.metric,
        aggregation: &p.aggregation,
        comparison_rule: &p.comparison_rule,
        step_budget: p.step_budget,
        unit: &p.unit,
    })
}

// ---------------------------------------------------------------------------
// Evaluation suite
// ---------------------------------------------------------------------------

/// The frozen set of cases a measurement ran over.
///
/// Replaces the *singular* `prompt_byte_identity` of the generation contract,
/// which can name exactly one prompt and so cannot describe a 20–50 prompt corpus.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvaluationSuiteTerms {
    pub suite_name: String,
    /// Per-case content digests. **SORTED** — a suite is a set, and the order
    /// cases happen to be listed in must not fork its identity.
    pub case_digests: Vec<String>,
    pub tokenizer_identity: String,
    pub context_cap: u32,
}

pub fn evaluation_suite_identity(s: &EvaluationSuiteTerms) -> String {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Doc<'a> {
        domain: &'static str,
        suite_name: &'a str,
        case_digests: Vec<String>,
        tokenizer_identity: &'a str,
        context_cap: u32,
    }
    let mut case_digests = s.case_digests.clone();
    case_digests.sort();
    seal(&Doc {
        domain: EVALUATION_SUITE_DOMAIN,
        suite_name: &s.suite_name,
        case_digests,
        tokenizer_identity: &s.tokenizer_identity,
        context_cap: s.context_cap,
    })
}

// ---------------------------------------------------------------------------
// Reference execution
// ---------------------------------------------------------------------------

/// The run a relative quantity was measured *against*.
///
/// The generation contract has no such field, which is why "maximum divergence"
/// there has no bound referent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReferenceExecutionTerms {
    pub runtime_identity: String,
    pub backend_id: String,
    pub artifact_sha256: String,
    pub evaluation_suite_identity: String,
    /// Host/OS/driver facts that would change the result. **SORTED** — a set of
    /// `key=value` strings, not an ordered log.
    pub environment: Vec<String>,
}

pub fn reference_execution_identity(r: &ReferenceExecutionTerms) -> String {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Doc<'a> {
        domain: &'static str,
        runtime_identity: &'a str,
        backend_id: &'a str,
        artifact_sha256: &'a str,
        evaluation_suite_identity: &'a str,
        environment: Vec<String>,
    }
    let mut environment = r.environment.clone();
    environment.sort();
    seal(&Doc {
        domain: REFERENCE_EXECUTION_DOMAIN,
        runtime_identity: &r.runtime_identity,
        backend_id: &r.backend_id,
        artifact_sha256: &r.artifact_sha256,
        evaluation_suite_identity: &r.evaluation_suite_identity,
        environment,
    })
}

// ---------------------------------------------------------------------------
// Observation
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum QuantityKind {
    /// Stands alone: a throughput, a byte count, a temperature.
    Absolute,
    /// Meaningless without naming what it is relative TO: a divergence, a
    /// speedup, a delta. Requires `reference_execution_identity`.
    Relative,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservationTerms {
    pub observation_kind: String,
    pub quantity_kind: QuantityKind,
    /// Canonical decimal text, NOT `f64`. See the float caveat in `digest.rs`:
    /// a float in a sealed document is at the mercy of the serializer's
    /// formatter, so the sealed form is the exact text that was measured.
    pub value_text: String,
    pub measurement_policy_identity: String,
    pub evaluation_suite_identity: String,
    /// Required when `quantity_kind` is `Relative`; forbidden otherwise.
    pub reference_execution_identity: Option<String>,
    pub runtime_identity: String,
    pub artifact_sha256: Option<String>,
}

/// Reasons an observation is not storable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ObservationDefect {
    /// A relative quantity with nothing to be relative to. This is the defect
    /// the GPD Vulkan work surfaced, made unrepresentable.
    RelativeWithoutReference,
    /// An absolute quantity claiming a reference it cannot use — visible rather
    /// than silently ignored, mirroring `MeasurementOutOfScope` upstream.
    AbsoluteWithReference,
    ValueNotDecimal {
        value_text: String,
    },
    MalformedDigest {
        field: &'static str,
    },
    EmptyField {
        field: &'static str,
    },
}

/// The pure-layer twin of the SQL `observation_relative_needs_reference` CHECK.
///
/// Enforced in both layers deliberately: the constraint is the point of the
/// store, and a rule that lives in only one place is a rule that can be bypassed
/// by writing through the other.
pub fn validate_observation(o: &ObservationTerms) -> Vec<ObservationDefect> {
    let mut errs = Vec::new();

    match (o.quantity_kind, o.reference_execution_identity.as_deref()) {
        (QuantityKind::Relative, None) => errs.push(ObservationDefect::RelativeWithoutReference),
        (QuantityKind::Absolute, Some(_)) => errs.push(ObservationDefect::AbsoluteWithReference),
        _ => {}
    }

    if o.value_text.trim().is_empty() || o.value_text.parse::<f64>().is_err() {
        errs.push(ObservationDefect::ValueNotDecimal {
            value_text: o.value_text.clone(),
        });
    }

    for (field, v) in [
        ("measurementPolicyIdentity", &o.measurement_policy_identity),
        ("evaluationSuiteIdentity", &o.evaluation_suite_identity),
    ] {
        if !valid_sha256(v) {
            errs.push(ObservationDefect::MalformedDigest { field });
        }
    }
    if let Some(r) = &o.reference_execution_identity {
        if !valid_sha256(r) {
            errs.push(ObservationDefect::MalformedDigest {
                field: "referenceExecutionIdentity",
            });
        }
    }
    if let Some(a) = &o.artifact_sha256 {
        if !valid_sha256(a) {
            errs.push(ObservationDefect::MalformedDigest {
                field: "artifactSha256",
            });
        }
    }
    for (field, v) in [
        ("observationKind", &o.observation_kind),
        ("runtimeIdentity", &o.runtime_identity),
    ] {
        if v.trim().is_empty() {
            errs.push(ObservationDefect::EmptyField { field });
        }
    }
    errs
}

pub fn observation_identity(o: &ObservationTerms) -> String {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Doc<'a> {
        domain: &'static str,
        observation_kind: &'a str,
        quantity_kind: QuantityKind,
        value_text: &'a str,
        measurement_policy_identity: &'a str,
        evaluation_suite_identity: &'a str,
        reference_execution_identity: Option<&'a str>,
        runtime_identity: &'a str,
        artifact_sha256: Option<&'a str>,
    }
    seal(&Doc {
        domain: OBSERVATION_DOMAIN,
        observation_kind: &o.observation_kind,
        quantity_kind: o.quantity_kind,
        value_text: &o.value_text,
        measurement_policy_identity: &o.measurement_policy_identity,
        evaluation_suite_identity: &o.evaluation_suite_identity,
        reference_execution_identity: o.reference_execution_identity.as_deref(),
        runtime_identity: &o.runtime_identity,
        artifact_sha256: o.artifact_sha256.as_deref(),
    })
}

// ---------------------------------------------------------------------------
// Artifact
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactTerms {
    pub artifact_kind: String,
    pub sha256_hex: String,
    pub byte_size: u64,
    pub media_type: String,
    pub source_uri: String,
}

pub fn artifact_identity(a: &ArtifactTerms) -> String {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Doc<'a> {
        domain: &'static str,
        artifact_kind: &'a str,
        sha256_hex: &'a str,
        byte_size: u64,
        media_type: &'a str,
        source_uri: &'a str,
    }
    seal(&Doc {
        domain: ARTIFACT_DOMAIN,
        artifact_kind: &a.artifact_kind,
        sha256_hex: &a.sha256_hex,
        byte_size: a.byte_size,
        media_type: &a.media_type,
        source_uri: &a.source_uri,
    })
}

// ---------------------------------------------------------------------------
// Memory record
// ---------------------------------------------------------------------------

/// A claim, with where it came from.
///
/// `record_digest` is the idempotency key: importing the same claim twice
/// produces one row and no new history.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryRecordTerms {
    pub claim: String,
    pub evidence_class: EvidenceClass,
    pub source_artifact_sha256: Option<String>,
    /// Where in the source, e.g. `§4 table row 1`.
    pub source_locator: Option<String>,
    /// Observations this claim rests on. **SORTED** — a set of supports.
    pub observation_identities: Vec<String>,
    pub harness_run_id: Option<String>,
}

pub fn memory_record_identity(m: &MemoryRecordTerms) -> String {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Doc<'a> {
        domain: &'static str,
        claim: &'a str,
        evidence_class: EvidenceClass,
        source_artifact_sha256: Option<&'a str>,
        source_locator: Option<&'a str>,
        observation_identities: Vec<String>,
        harness_run_id: Option<&'a str>,
    }
    let mut observation_identities = m.observation_identities.clone();
    observation_identities.sort();
    seal(&Doc {
        domain: MEMORY_RECORD_DOMAIN,
        claim: &m.claim,
        evidence_class: m.evidence_class,
        source_artifact_sha256: m.source_artifact_sha256.as_deref(),
        source_locator: m.source_locator.as_deref(),
        observation_identities,
        harness_run_id: m.harness_run_id.as_deref(),
    })
}

// ---------------------------------------------------------------------------
// Embedding space
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Pooling {
    Mean,
    Cls,
    LastToken,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Normalization {
    None,
    L2,
}

/// What makes two vectors comparable.
///
/// Field set mirrors `neuralcompose-mobile-core`'s `embedding_space_identity`
/// (`model_pack.rs:305`), deliberately including pooling, normalization and the
/// task instruction — change any of them and the vectors mean something else,
/// however similar the text.
///
/// **The backend is absent, and that is the point.** A CPU run and an NPU run of
/// the same model produce vectors in the same space or they do not, and that is
/// a question to be *measured*, not asserted by stamping a different identity on
/// them. Putting the backend here would fork the space by declaration and make
/// the measurement unaskable. Where the vector came from belongs on the runtime
/// variant; whether the two may share an index is what conformance decides.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingProfileTerms {
    pub model_family: String,
    pub model_revision: String,
    /// Weight artifact digests. **SORTED** — shard listing order is noise.
    pub weight_sha256: Vec<String>,
    /// Tokenizer artifact digests. **SORTED.**
    pub tokenizer_sha256: Vec<String>,
    pub dimensions: u32,
    pub pooling: Pooling,
    pub normalization: Normalization,
    /// e.g. nomic's `search_document:` / `search_query:` prefixes. A different
    /// instruction is a different space, not a different query.
    pub task_instruction: Option<String>,
}

const EMBEDDING_SPACE_DOMAIN: &str = "neuralmemory.embedding-space.v1";

pub fn embedding_space_identity(p: &EmbeddingProfileTerms) -> String {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Doc<'a> {
        domain: &'static str,
        model_family: &'a str,
        model_revision: &'a str,
        weight_sha256: Vec<String>,
        tokenizer_sha256: Vec<String>,
        dimensions: u32,
        pooling: Pooling,
        normalization: Normalization,
        task_instruction: Option<&'a str>,
    }
    let mut w = p.weight_sha256.clone();
    w.sort();
    let mut t = p.tokenizer_sha256.clone();
    t.sort();
    seal(&Doc {
        domain: EMBEDDING_SPACE_DOMAIN,
        model_family: &p.model_family,
        model_revision: &p.model_revision,
        weight_sha256: w,
        tokenizer_sha256: t,
        dimensions: p.dimensions,
        pooling: p.pooling,
        normalization: p.normalization,
        task_instruction: p.task_instruction.as_deref(),
    })
}

/// One indexed record: *what* was embedded and *by which embedding space*.
///
/// Carried over from `property_law.rs:29`. Two records with the same pair are
/// the same entry; two with different space identities are never the same entry,
/// however similar the text.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexEntryKey {
    pub content_sha256_hex: String,
    pub embedding_space_identity: String,
}

/// Do these two entries belong in the same index?
///
/// Only when the embedding space is identical — **mixing spaces silently poisons
/// retrieval**. Silently is the operative word: cosine similarity between
/// vectors from different spaces returns a plausible number, so nothing fails,
/// results are merely wrong.
pub fn shares_index(a: &IndexEntryKey, b: &IndexEntryKey) -> bool {
    a.embedding_space_identity == b.embedding_space_identity
}
