//! Bounded import of the GPD characterization evidence. **Operator channel.**
//!
//! Scope is fixed and deliberately small, because turning a hand-written
//! document with tables, four confidence-marker classes, prose caveats and two
//! supersessions into typed rows is an unbounded transcription task with no
//! natural stopping point. The bound is:
//!
//!   - the section 4 conformance table
//!   - the section 1 superseded 14B row and its replacement
//!   - the section 12 open items
//!   - the ADR-002 / schema excerpts the H6 rubric requires
//!
//! Anything else is out of scope for M1. If this file grows past that, stop and
//! reassess rather than widening it.
//!
//! ## The linking prose is deliberately not imported
//!
//! Each record is one atomic claim. No stored record contains the whole
//! explanation, so answering the H6 question requires actually joining facts
//! across records rather than retrieving one well-written paragraph. Importing
//! the connective prose would make the test pass while measuring nothing.
//!
//! ## Retired claims are imported AS retired
//!
//! The document's superseded claims are imported along with their replacements
//! and a `supersedes` edge. Omitting them would remove the decoy that makes H6
//! discriminating; importing them as unmarked peers would poison retrieval with
//! contradictions, which is the exact failure this store exists to prevent.

use neural_memory_domain::*;
use neural_memory_store::*;

const AT: &str = "2026-07-30T12:00:00Z";
const RUNTIME: &str = "llama.cpp-b10188-d0bfb1981";

// Real digests, taken from the files on disk at import time.
const DOC_SHA: &str = "381918cf16b054a7924f4b02acc581b96e181eed0dd1dc7a004273371363e905";
const ADR_SHA: &str = "f4f33e165fa98f7ff1b36b4ec49b5070bbb63108ebcd7c64884df5ab60b89297";
const SCHEMA_SHA: &str = "4e8c76a0a3475c4d3b096cab0ade467dfceae40af4ff194604c91fbf4e2a26f6";
const CONFORMANCE_SHA: &str = "eb7cb908f041befba5f396839d783722e7cf144685da52b504cfd8f12f6d16a6";

struct Import<'a> {
    store: &'a Store,
    inserted: usize,
    present: usize,
}

impl Import<'_> {
    /// A claim resting on observations, from a named source location.
    fn claim(
        &mut self,
        text: &str,
        class: EvidenceClass,
        artifact: Option<&str>,
        locator: &str,
        obs: Vec<String>,
    ) -> Result<String, String> {
        let w = MemoryWrite {
            terms: MemoryRecordTerms {
                claim: text.into(),
                evidence_class: class,
                source_artifact_sha256: artifact.map(str::to_string),
                source_locator: Some(locator.into()),
                observation_identities: obs,
                harness_run_id: Some("gpd-characterization-20260730".into()),
            },
            occurred_at: Some(AT),
            recorded_at: Some(AT),
            derivation: None,
        };
        let (d, wrote) = self
            .store
            .put_memory(WriteChannel::Operator, &w)
            .map_err(|e| format!("{text:.60}...: {e}"))?;
        if wrote.inserted() {
            self.inserted += 1;
        } else {
            self.present += 1;
        }
        Ok(d)
    }
}

pub fn import_gpd(store: &Store) -> Result<String, String> {
    let mut im = Import {
        store,
        inserted: 0,
        present: 0,
    };

    // ---- artifacts ------------------------------------------------------
    for (sha, kind, bytes, media, uri) in [
        (DOC_SHA, "characterization-document", 27512u64, "text/markdown",
         "file:///home/aurascoper/Downloads/gpd-vulkan-characterization-20260730.md"),
        (ADR_SHA, "architecture-decision-record", 7561, "text/markdown",
         "file:///home/aurascoper/src/neuralcompose-client-native/docs/architecture/decision-log/ADR-002-runtime-targets-property-law-and-conformance.md"),
        (SCHEMA_SHA, "json-schema", 930, "application/schema+json",
         "file:///home/aurascoper/src/neuralcompose-client-native/contracts/runtime/model-variant.schema.json"),
        (CONFORMANCE_SHA, "source-file", 13637, "text/x-rust",
         "file:///home/aurascoper/src/neuralcompose-client-native/crates/neuralcompose-mobile-core/src/conformance.rs"),
    ] {
        store
            .put_artifact(
                &ArtifactTerms {
                    artifact_kind: kind.into(),
                    sha256_hex: sha.into(),
                    byte_size: bytes,
                    media_type: media.into(),
                    source_uri: uri.into(),
                },
                AT,
            )
            .map_err(|e| e.to_string())?;
    }

    // ---- the section 4 measurement apparatus ----------------------------
    let (suite4, _) = store
        .put_evaluation_suite(&EvaluationSuiteTerms {
            suite_name: "gpd-single-prompt-greedy".into(),
            // One case. That single fact is what F6 ultimately rests on.
            case_digests: vec![sha256_hex(b"gpd section 4 prompt, no chat template")],
            tokenizer_identity: "per-model-native".into(),
            context_cap: 8192,
        })
        .map_err(|e| e.to_string())?;

    let (policy_logit, _) = store
        .put_measurement_policy(&MeasurementPolicyTerms {
            metric: "maxAbsoluteLogitDelta".into(),
            aggregation: "maxOverPreDivergenceSteps".into(),
            comparison_rule: "lessThanOrEqualTolerance".into(),
            step_budget: Some(58),
            unit: "logit".into(),
        })
        .map_err(|e| e.to_string())?;

    let (cpu_ref, _) = store
        .put_reference_execution(&ReferenceExecutionTerms {
            runtime_identity: RUNTIME.into(),
            backend_id: "llama-cpp-cpu".into(),
            artifact_sha256: CONFORMANCE_SHA.into(),
            evaluation_suite_identity: suite4.clone(),
            environment: vec![
                "os=ubuntu-26.04".into(),
                "kernel=7.0.0-28-generic".into(),
                "governor=performance".into(),
                "ac=1".into(),
            ],
        })
        .map_err(|e| e.to_string())?;

    let relative = |kind: &str, value: &str| -> Result<String, String> {
        store
            .put_observation(
                &ObservationTerms {
                    observation_kind: kind.into(),
                    quantity_kind: QuantityKind::Relative,
                    value_text: value.into(),
                    measurement_policy_identity: policy_logit.clone(),
                    evaluation_suite_identity: suite4.clone(),
                    reference_execution_identity: Some(cpu_ref.clone()),
                    runtime_identity: RUNTIME.into(),
                    artifact_sha256: Some(DOC_SHA.into()),
                },
                AT,
            )
            .map(|(id, _)| id)
            .map_err(|e| e.to_string())
    };
    let obs_gemma_delta = relative("maxLogitDivergence.gemma4-12b-q5km.vulkan", "4.3362")?;
    let obs_qwen_delta = relative("maxLogitDivergence.qwen3-8b-q6k.vulkan", "2.3901")?;

    // ---- section 4 claims, each atomic ----------------------------------
    let f1 = im.claim(
        "Gemma 4 12B Q5_K_M on Vulkan reached a maximum absolute logit delta of 4.3362 against the CPU reference execution",
        EvidenceClass::Observed, Some(DOC_SHA), "§4 table", vec![obs_gemma_delta],
    )?;
    let qwen_delta = im.claim(
        "Qwen3 8B Q6_K on Vulkan reached a maximum absolute logit delta of 2.3901 against the CPU reference execution",
        EvidenceClass::Observed, Some(DOC_SHA), "§4 table", vec![obs_qwen_delta],
    )?;
    let f2 = im.claim(
        "The logits tolerance of 0.5 was pre-registered on principle before any logits were captured",
        EvidenceClass::HumanDecision, Some(DOC_SHA), "§4", vec![],
    )?;
    let f5 = im.claim(
        "The section 4 conformance measurement used a single prompt with greedy decoding and no chat template, over 32 steps for Qwen3-8B and 58 steps for Gemma 4 12B",
        EvidenceClass::Observed, Some(DOC_SHA), "§4 marker", vec![],
    )?;
    let f6 = im.claim(
        "Divergence maxima taken from a single trajectory are lower bounds rather than characterised distributions, so no defensible Vulkan tolerance can yet be pre-registered",
        EvidenceClass::ExternalClaim, Some(DOC_SHA), "§4 amendment / §12", vec![],
    )?;
    let step52 = im.claim(
        "Gemma 4 12B Q5_K_M first diverged from the CPU reference at greedy step 52, having appeared identical at step 32",
        EvidenceClass::Observed, Some(DOC_SHA), "§4 table", vec![],
    )?;

    // ---- ADR-002 and schema excerpts ------------------------------------
    let f3 = im.claim(
        "ADR-002 decision 6 states that exceeding a declared tolerance is a different contract, not a failed test",
        EvidenceClass::Observed, Some(ADR_SHA), "ADR-002 decision 6", vec![],
    )?;
    let f4 = im.claim(
        "A backend whose verdict is RequiresSeparateNumericalContract publishes under its own numericalContractId rather than the reference backend's",
        EvidenceClass::Observed, Some(ADR_SHA), "ADR-002 decision 6", vec![],
    )?;
    let f8 = im.claim(
        "model-variant.schema.json lists numericalContractId among its required properties, so a variant cannot be emitted with the field absent",
        EvidenceClass::Observed, Some(SCHEMA_SHA), "required[]", vec![],
    )?;
    let binds = im.claim(
        "variant_binds_to_contract compares a variant's id to the seal of a policy and never consults a ConformanceVerdict",
        EvidenceClass::Observed, Some(CONFORMANCE_SHA), "conformance.rs:171", vec![],
    )?;

    // ---- supersession 1: the retired root-cause claim --------------------
    // The decoy. Imported AS RETIRED so an agent that pattern-matches the old
    // wording can still find it -- and can also find that it was retired.
    let f7_retired = im.claim(
        "Vulkan variants are blocked because BackendConformancePolicy has no backend dimension, so identical terms necessarily yield an identical seal",
        EvidenceClass::ExternalClaim, Some(DOC_SHA), "§4 (superseded 2026-07-30)", vec![],
    )?;
    let f7_replacement = im.claim(
        "The Vulkan contract is blocked by the measurement corpus rather than by the schema: the binding path is mechanically open and the missing piece is a defensible pre-registered tolerance",
        EvidenceClass::ExternalClaim, Some(DOC_SHA), "§4 amendment 2026-07-30", vec![],
    )?;
    store
        .supersede(&f7_retired, &f7_replacement, AT)
        .map_err(|e| e.to_string())?;

    // ---- supersession 2: the section 1 battery figures -------------------
    let (policy_pp, _) = store
        .put_measurement_policy(&MeasurementPolicyTerms {
            metric: "promptTokensPerSecond".into(),
            aggregation: "meanOfFiveRepetitions".into(),
            comparison_rule: "reportOnly".into(),
            step_budget: Some(5),
            unit: "tokensPerSecond".into(),
        })
        .map_err(|e| e.to_string())?;
    let (suite_bench, _) = store
        .put_evaluation_suite(&EvaluationSuiteTerms {
            suite_name: "llama-bench-pp512-tg128".into(),
            case_digests: vec![sha256_hex(b"-p 512 -n 128 -ctk q8_0 -ctv q8_0 -r 5")],
            tokenizer_identity: "per-model-native".into(),
            context_cap: 8192,
        })
        .map_err(|e| e.to_string())?;
    let absolute = |kind: &str, value: &str| -> Result<String, String> {
        store
            .put_observation(
                &ObservationTerms {
                    observation_kind: kind.into(),
                    quantity_kind: QuantityKind::Absolute,
                    value_text: value.into(),
                    measurement_policy_identity: policy_pp.clone(),
                    evaluation_suite_identity: suite_bench.clone(),
                    reference_execution_identity: None,
                    runtime_identity: RUNTIME.into(),
                    artifact_sha256: Some(DOC_SHA.into()),
                },
                AT,
            )
            .map(|(id, _)| id)
            .map_err(|e| e.to_string())
    };
    let o_batt = absolute("pp512.qwen3-14b-q4km.vulkan.battery", "159.96")?;
    let o_ac = absolute("pp512.qwen3-14b-q4km.vulkan.ac", "147.91")?;

    let batt = im.claim(
        "Qwen3 14B Q4_K_M reached 159.96 prompt tokens per second on Vulkan while on battery",
        EvidenceClass::Observed,
        Some(DOC_SHA),
        "§1 (superseded)",
        vec![o_batt],
    )?;
    let ac = im.claim(
        "Qwen3 14B Q4_K_M reached 147.91 prompt tokens per second on Vulkan under AC power",
        EvidenceClass::Observed,
        Some(DOC_SHA),
        "§1",
        vec![o_ac],
    )?;
    let no_rescale = im.claim(
        "The AC and battery prompt-processing figures do not differ by a uniform offset, so battery numbers must be retired rather than rescaled",
        EvidenceClass::ExternalClaim, Some(DOC_SHA), "§1", vec![],
    )?;
    store.supersede(&batt, &ac, AT).map_err(|e| e.to_string())?;

    // ---- section 12 open items ------------------------------------------
    let open_corpus = im.claim(
        "A conformance corpus of 20 to 50 frozen prompts is required before any durable interchangeability claim",
        EvidenceClass::ExternalClaim, Some(DOC_SHA), "§12", vec![],
    )?;
    let latent_gap = im.claim(
        "The conformance schema names no reference execution, evaluation suite or measurement policy, which is a real but latent gap rather than the cause of the block",
        EvidenceClass::ExternalClaim, Some(DOC_SHA), "§12", vec![],
    )?;

    // ---- provenance -----------------------------------------------------
    // Edges connect facts that must be joined to answer the question. The
    // connective PROSE is not imported; these edges are the only linkage, so a
    // correct answer has to traverse rather than quote.
    for (src, dst, kind) in [
        (&f1, &f2, "derivedFrom"),
        (&f1, &f3, "derivedFrom"),
        (&f3, &f4, "supports"),
        (&f5, &f6, "supports"),
        (&f6, &open_corpus, "supports"),
        (&f7_replacement, &f6, "derivedFrom"),
        (&f7_replacement, &binds, "supports"),
        (&f4, &f8, "supports"),
        (&qwen_delta, &f2, "derivedFrom"),
        (&step52, &f5, "supports"),
        (&latent_gap, &f6, "contradicts"),
        (&batt, &no_rescale, "supports"),
        (&ac, &no_rescale, "supports"),
    ] {
        store
            .add_edge(src, dst, kind, AT)
            .map_err(|e| e.to_string())?;
    }

    Ok(format!(
        "import complete: {} claims inserted, {} already present, maxSeq={}, integrity={}",
        im.inserted,
        im.present,
        store.max_recorded_seq().map_err(|e| e.to_string())?,
        store.integrity_ok().map_err(|e| e.to_string())?
    ))
}
