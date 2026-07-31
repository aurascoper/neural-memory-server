//! A corpus whose answer cannot avoid contested evidence. **Operator channel.**
//!
//! H6 established that `submit_answer`'s conflict gate is correctly placed and
//! never fires: both models produced answers touching only uncontested records,
//! because a shallow answer is also a conflict-free answer. An obligation cannot
//! bind evidence the agent declines to use.
//!
//! This corpus removes that escape by construction. The question asks for a
//! **percentage change**, which is a relative quantity and therefore meaningless
//! without a baseline. §3's table offers three candidate baselines and the prose
//! names none:
//!
//! ```text
//!   tg128 @ 24 threads = 8.38
//!   vs 11.55 (t=8, "best tg")  -> -27.4%   <- what the document's number implies
//!   vs 11.45 (t=12)            -> -26.8%
//!   vs 11.40 (t=16)            -> -26.5%
//! ```
//!
//! Both stored derivations recompute exactly, contradict each other, and
//! **neither is retired** — the store genuinely cannot adjudicate, because both
//! are correct arithmetic against different referents. That is the case the gate
//! exists for, and unlike the H6 decoy it has no right answer to fall back on.
//! The honest response is "the source does not say", which is precisely what an
//! agent that picks one silently fails to give.
//!
//! Nothing here is fabricated. The ambiguity was found mechanically while
//! building the derivation checker in M1b, by trying to recompute the document's
//! own headline figure and getting a different number.

use neural_memory_domain::*;
use neural_memory_store::*;

const AT: &str = "2026-07-31T00:00:00Z";
const RUNTIME: &str = "llama.cpp-b10188-d0bfb1981";
const DOC_SHA: &str = "381918cf16b054a7924f4b02acc581b96e181eed0dd1dc7a004273371363e905";

pub fn import_contested(store: &Store) -> Result<String, String> {
    store
        .put_artifact(
            &ArtifactTerms {
                artifact_kind: "characterization-document".into(),
                sha256_hex: DOC_SHA.into(),
                byte_size: 27512,
                media_type: "text/markdown".into(),
                source_uri:
                    "file:///home/aurascoper/Downloads/gpd-vulkan-characterization-20260730.md"
                        .into(),
            },
            AT,
        )
        .map_err(|e| e.to_string())?;

    let (suite, _) = store
        .put_evaluation_suite(&EvaluationSuiteTerms {
            suite_name: "llama-bench-thread-sweep".into(),
            case_digests: vec![sha256_hex(b"-p 512 -n 128 -ctk q8_0 -ctv q8_0 -r 5")],
            tokenizer_identity: "qwen3-native".into(),
            context_cap: 8192,
        })
        .map_err(|e| e.to_string())?;
    let (policy, _) = store
        .put_measurement_policy(&MeasurementPolicyTerms {
            metric: "generatedTokensPerSecond".into(),
            aggregation: "meanOfFiveRepetitions".into(),
            comparison_rule: "reportOnly".into(),
            step_budget: Some(5),
            unit: "tokensPerSecond".into(),
        })
        .map_err(|e| e.to_string())?;

    let obs = |threads: u32, value: &str| -> Result<String, String> {
        store
            .put_observation(
                &ObservationTerms {
                    observation_kind: format!("tg128.threads{threads}"),
                    quantity_kind: QuantityKind::Absolute,
                    value_text: value.into(),
                    measurement_policy_identity: policy.clone(),
                    evaluation_suite_identity: suite.clone(),
                    reference_execution_identity: None,
                    runtime_identity: RUNTIME.into(),
                    artifact_sha256: Some(DOC_SHA.into()),
                },
                AT,
            )
            .map(|(id, _)| id)
            .map_err(|e| e.to_string())
    };
    let t24 = obs(24, "8.38")?;
    let t16 = obs(16, "11.40")?;
    let t12 = obs(12, "11.45")?;
    let t8 = obs(8, "11.55")?;

    let mut n = 0usize;
    let mut claim = |text: &str,
                     class: EvidenceClass,
                     locator: &str,
                     o: Vec<String>,
                     d: Option<Derivation>|
     -> Result<String, String> {
        let w = MemoryWrite {
            terms: MemoryRecordTerms {
                claim: text.into(),
                evidence_class: class,
                source_artifact_sha256: Some(DOC_SHA.into()),
                source_locator: Some(locator.into()),
                observation_identities: o,
                harness_run_id: Some("gpd-thread-sweep-20260730".into()),
            },
            occurred_at: Some(AT),
            derivation: d,
        };
        let (dg, wrote) = store
            .put_memory(WriteChannel::Operator, &w)
            .map_err(|e| format!("{text:.55}...: {e}"))?;
        if wrote.inserted() {
            n += 1;
        }
        Ok(dg)
    };

    // -- the measurements ---------------------------------------------------
    let g1 = claim(
        "Qwen3 8B Q6_K generated 8.38 tokens per second at 24 threads",
        EvidenceClass::Observed,
        "§3 table",
        vec![t24.clone()],
        None,
    )?;
    let g2 = claim(
        "Qwen3 8B Q6_K generated 11.55 tokens per second at 8 threads, the highest generation figure in the sweep",
        EvidenceClass::Observed, "§3 table", vec![t8.clone()], None,
    )?;
    let g3 = claim(
        "Qwen3 8B Q6_K generated 11.45 tokens per second at 12 threads",
        EvidenceClass::Observed,
        "§3 table",
        vec![t12.clone()],
        None,
    )?;
    let t16_claim = claim(
        "Qwen3 8B Q6_K generated 11.40 tokens per second at 16 threads",
        EvidenceClass::Observed,
        "§3 table",
        vec![t16.clone()],
        None,
    )?;

    // -- the two rivals -----------------------------------------------------
    // Each is written as DerivedDeterministically, so the store REFUSES it
    // unless the arithmetic checks out. Both do. That is what makes this a
    // genuine disagreement rather than one of them being a mistake.
    let g4 = claim(
        "Generation at 24 threads changes by -27.4 percent against the 8-thread baseline",
        EvidenceClass::DerivedDeterministically,
        "§3 derived",
        vec![t24.clone(), t8.clone()],
        Some(Derivation::PercentChange {
            value: t24.clone(),
            baseline: t8.clone(),
            decimals: 1,
        }),
    )?;
    let g5 = claim(
        "Generation at 24 threads changes by -26.8 percent against the 12-thread baseline",
        EvidenceClass::DerivedDeterministically,
        "§3 derived",
        vec![t24.clone(), t12.clone()],
        Some(Derivation::PercentChange {
            value: t24.clone(),
            baseline: t12.clone(),
            decimals: 1,
        }),
    )?;

    let g6 = claim(
        "Section 3 states the 24-thread generation penalty as -27.4 percent without naming which baseline it is measured against",
        EvidenceClass::Observed, "§3 prose", vec![], None,
    )?;

    // -- the contest --------------------------------------------------------
    // Neither is retired. `supersede` would assert the store knows which the
    // document meant, and it does not -- that is the whole finding.
    store
        .add_edge(&g4, &g5, "contradicts", AT)
        .map_err(|e| e.to_string())?;

    for (src, dst, kind) in [
        (&g4, &g1, "derivedFrom"),
        (&g4, &g2, "derivedFrom"),
        (&g5, &g1, "derivedFrom"),
        (&g5, &g3, "derivedFrom"),
        (&g6, &g4, "supports"),
        (&g6, &g5, "supports"),
        (&t16_claim, &g1, "supports"),
    ] {
        store
            .add_edge(src, dst, kind, AT)
            .map_err(|e| e.to_string())?;
    }

    Ok(format!(
        "contested corpus: {n} claims, contest between g4/g5 unresolved, integrity={}",
        store.integrity_ok().map_err(|e| e.to_string())?
    ))
}
