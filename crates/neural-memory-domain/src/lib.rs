//! Pure domain core for the NeuralCompose memory and evidence store.
//!
//! **Effect-free by construction.** This crate opens no file, reads no clock,
//! resolves no host, and links no database. Timestamps and token counts are
//! supplied by the caller. The doctrine is carried over from
//! `neuralcompose-mobile-core`, where `lib.rs:4-10` states it as *"Shells own all
//! I/O … this crate never reads a clock."*
//!
//! It is enforced mechanically rather than by intent: `scripts/check-purity.sh`
//! asserts that `rusqlite`, `tokio` and `uuid` are absent from this crate's
//! dependency tree. That matters for one rule in particular — **a UUID must never
//! be mistaken for an identity.** Primary keys are foreign-key targets; the
//! 64-hex seal is the identity. If this crate could mint a UUID, the two could be
//! confused with nothing failing.
//!
//! ## What this crate is for
//!
//! Not latency. A measured experiment (see `assembler`) falsified the premise
//! that retrieval reduces time-to-first-token on this hardware. What survives is
//! the set of things prose plus `grep` genuinely cannot do:
//!
//! - **addressable units**, so "cite record X" is mechanically checkable;
//! - **supersession that retrieval respects**, so a retired claim does not come
//!   back with equal standing to its replacement;
//! - **evidence class derived from structure**, never asserted by the writer;
//! - **a referent constraint that is enforced**, so a relative quantity with no
//!   named reference is unrepresentable rather than merely discouraged.

pub mod assembler;
pub mod digest;
pub mod terms;

pub use assembler::{
    plan_append, plan_supersession, session_prefix_identity, simulate_append_only,
    simulate_rebuild_per_turn, AppendPlan, AssemblerConfig, SessionCost, SessionPrefix,
};
pub use digest::{sha256_hex, valid_sha256};
pub use terms::{
    artifact_identity, embedding_space_identity, evaluation_suite_identity,
    measurement_policy_identity, memory_record_identity, observation_identity,
    reference_execution_identity, shares_index, validate_observation, ArtifactTerms,
    EmbeddingProfileTerms, EvaluationSuiteTerms, EvidenceClass, IndexEntryKey,
    MeasurementPolicyTerms, MemoryRecordTerms, Normalization, ObservationDefect, ObservationTerms,
    Pooling, QuantityKind, ReferenceExecutionTerms,
};
