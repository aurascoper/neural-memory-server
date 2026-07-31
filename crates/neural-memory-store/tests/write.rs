//! Idempotency and evidence-class derivation.
//!
//! Both polarities throughout: a gate that refuses everything passes a
//! refusal test, and a gate that accepts everything passes an acceptance test.
//! Only the pair says anything.

use neural_memory_domain::*;
use neural_memory_store::*;

const NOW: &str = "2026-07-30T12:00:00Z";

fn art() -> ArtifactTerms {
    ArtifactTerms {
        artifact_kind: "characterization-doc".into(),
        sha256_hex: "aa".repeat(32),
        byte_size: 25061,
        media_type: "text/markdown".into(),
        source_uri: "file:///home/aurascoper/Downloads/gpd.md".into(),
    }
}

fn suite() -> EvaluationSuiteTerms {
    EvaluationSuiteTerms {
        suite_name: "gpd-bench".into(),
        case_digests: vec!["bb".repeat(32)],
        tokenizer_identity: "qwen3-tok".into(),
        context_cap: 8192,
    }
}

fn policy(metric: &str) -> MeasurementPolicyTerms {
    MeasurementPolicyTerms {
        metric: metric.into(),
        aggregation: "meanOfFiveRepetitions".into(),
        comparison_rule: "reportOnly".into(),
        step_budget: Some(5),
        unit: "tokensPerSecond".into(),
    }
}

/// A store seeded with the two §1 throughput observations the "7.27x" ratio is
/// actually derived from.
fn seeded() -> (Store, String, String) {
    let s = Store::open_in_memory().unwrap();
    s.put_artifact(&art(), NOW).unwrap();
    let (suite_id, _) = s.put_evaluation_suite(&suite()).unwrap();
    let (pol_id, _) = s
        .put_measurement_policy(&policy("promptTokensPerSecond"))
        .unwrap();

    let mk = |value: &str, backend: &str| ObservationTerms {
        observation_kind: format!("pp512.{backend}"),
        quantity_kind: QuantityKind::Absolute,
        value_text: value.into(),
        measurement_policy_identity: pol_id.clone(),
        evaluation_suite_identity: suite_id.clone(),
        reference_execution_identity: None,
        runtime_identity: "llama.cpp-b10188-d0bfb1981".into(),
        artifact_sha256: None,
    };
    let (vk, _) = s.put_observation(&mk("287.17", "vulkan"), NOW).unwrap();
    let (cpu, _) = s.put_observation(&mk("39.52", "cpu"), NOW).unwrap();
    (s, vk, cpu)
}

// ---------------------------------------------------------------------------
// Idempotency
// ---------------------------------------------------------------------------

#[test]
fn re_running_an_import_adds_no_rows_and_no_history() {
    let (s, vk, cpu) = seeded();

    let terms = MemoryRecordTerms {
        claim: "Vulkan reaches 287.17 prompt tokens per second".into(),
        evidence_class: EvidenceClass::Observed,
        source_artifact_sha256: Some(art().sha256_hex),
        source_locator: Some("§1".into()),
        observation_identities: vec![vk.clone(), cpu.clone()],
        harness_run_id: None,
    };
    let w = MemoryWrite {
        terms,
        occurred_at: Some(NOW),
        recorded_at: Some(NOW),
        derivation: None,
    };

    let (d1, first) = s.put_memory(WriteChannel::Operator, &w).unwrap();
    assert_eq!(first, Wrote::Inserted);
    let seq_after_first = s.max_recorded_seq().unwrap();

    let (d2, second) = s.put_memory(WriteChannel::Operator, &w).unwrap();
    assert_eq!(d1, d2, "the same claim must seal to the same digest");
    assert_eq!(
        second,
        Wrote::AlreadyPresent,
        "the caller is told it was already there rather than left to guess"
    );

    assert_eq!(
        s.max_recorded_seq().unwrap(),
        seq_after_first,
        "recorded_seq must not advance: a re-import is not new history"
    );
    let n: i64 = s
        .conn
        .query_row("SELECT count(*) FROM memories", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 1);

    // The observation links are idempotent too, or a re-import would multiply them.
    let links: i64 = s
        .conn
        .query_row("SELECT count(*) FROM memory_observations", [], |r| r.get(0))
        .unwrap();
    assert_eq!(links, 2);
}

#[test]
fn reference_data_writes_are_idempotent_and_report_which_happened() {
    let s = Store::open_in_memory().unwrap();
    assert_eq!(s.put_artifact(&art(), NOW).unwrap().1, Wrote::Inserted);
    assert_eq!(
        s.put_artifact(&art(), NOW).unwrap().1,
        Wrote::AlreadyPresent
    );

    assert_eq!(s.put_evaluation_suite(&suite()).unwrap().1, Wrote::Inserted);
    assert_eq!(
        s.put_evaluation_suite(&suite()).unwrap().1,
        Wrote::AlreadyPresent
    );

    // Polarity: a genuinely different policy is a different row.
    assert_eq!(
        s.put_measurement_policy(&policy("a")).unwrap().1,
        Wrote::Inserted
    );
    assert_eq!(
        s.put_measurement_policy(&policy("b")).unwrap().1,
        Wrote::Inserted
    );
}

#[test]
fn a_different_claim_is_a_different_record() {
    let (s, vk, _) = seeded();
    let mk = |claim: &str| MemoryWrite {
        terms: MemoryRecordTerms {
            claim: claim.into(),
            evidence_class: EvidenceClass::ExternalClaim,
            source_artifact_sha256: None,
            source_locator: None,
            observation_identities: vec![vk.clone()],
            harness_run_id: None,
        },
        occurred_at: None,
        recorded_at: None,
        derivation: None,
    };
    let (a, _) = s.put_memory(WriteChannel::Operator, &mk("one")).unwrap();
    let (b, _) = s.put_memory(WriteChannel::Operator, &mk("two")).unwrap();
    assert_ne!(a, b);
    assert_eq!(s.max_recorded_seq().unwrap(), 2);
}

// ---------------------------------------------------------------------------
// Evidence class is derived, not accepted
// ---------------------------------------------------------------------------

#[test]
fn an_agent_cannot_write_anything_above_agent_inference() {
    let (s, vk, _) = seeded();

    // The agent asks for `Observed`, with a real artifact, and would otherwise
    // qualify. It still does not get it.
    let w = MemoryWrite {
        terms: MemoryRecordTerms {
            claim: "I observed that Vulkan is faster".into(),
            evidence_class: EvidenceClass::Observed,
            source_artifact_sha256: Some(art().sha256_hex),
            source_locator: None,
            observation_identities: vec![vk],
            harness_run_id: None,
        },
        occurred_at: None,
        recorded_at: None,
        derivation: None,
    };
    let (digest, _) = s.put_memory(WriteChannel::Agent, &w).unwrap();

    let class: String = s
        .conn
        .query_row(
            "SELECT evidence_class FROM memories WHERE record_digest = ?1",
            [&digest],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(class, "agentInference");

    // And the clamp happens BEFORE sealing, so the digest is not even the same
    // record the agent asked for -- the higher-trust one is unrepresentable
    // through this door, not merely rejected at it.
    let operator_digest = memory_record_identity(&w.terms);
    assert_ne!(digest, operator_digest);
}

#[test]
fn an_operator_may_write_observed_when_the_artifact_exists() {
    let (s, _, _) = seeded();
    let w = MemoryWrite {
        terms: MemoryRecordTerms {
            claim: "The doc records pp512 = 287.17".into(),
            evidence_class: EvidenceClass::Observed,
            source_artifact_sha256: Some(art().sha256_hex),
            source_locator: Some("§1".into()),
            observation_identities: vec![],
            harness_run_id: None,
        },
        occurred_at: None,
        recorded_at: None,
        derivation: None,
    };
    assert!(s.put_memory(WriteChannel::Operator, &w).is_ok());
}

#[test]
fn observed_without_an_artifact_is_refused() {
    let (s, _, _) = seeded();
    let w = MemoryWrite {
        terms: MemoryRecordTerms {
            claim: "Vulkan is faster".into(),
            evidence_class: EvidenceClass::Observed,
            source_artifact_sha256: None,
            source_locator: None,
            observation_identities: vec![],
            harness_run_id: None,
        },
        occurred_at: None,
        recorded_at: None,
        derivation: None,
    };
    match s.put_memory(WriteChannel::Operator, &w) {
        Err(WriteError::Evidence(EvidenceRefusal::ObservedNeedsArtifact)) => {}
        other => panic!("expected refusal, got {other:?}"),
    }
}

#[test]
fn observed_citing_an_unknown_artifact_is_refused() {
    let (s, _, _) = seeded();
    let w = MemoryWrite {
        terms: MemoryRecordTerms {
            claim: "x".into(),
            evidence_class: EvidenceClass::Observed,
            source_artifact_sha256: Some("cc".repeat(32)),
            source_locator: None,
            observation_identities: vec![],
            harness_run_id: None,
        },
        occurred_at: None,
        recorded_at: None,
        derivation: None,
    };
    assert!(matches!(
        s.put_memory(WriteChannel::Operator, &w),
        Err(WriteError::Evidence(
            EvidenceRefusal::ObservedArtifactUnknown { .. }
        ))
    ));
}

// ---------------------------------------------------------------------------
// Deterministic derivation -- recomputed, not believed
// ---------------------------------------------------------------------------

#[test]
fn the_documented_vulkan_speedup_recomputes_from_its_two_inputs() {
    // §1 states "Vulkan speedup vs best CPU thread count: Qwen3-8B 7.27x pp".
    // That is not an observation -- it is 287.17 / 39.52. Recording it as
    // Observed would launder a derivation into a measurement.
    let (s, vk, cpu) = seeded();
    let w = MemoryWrite {
        terms: MemoryRecordTerms {
            claim: "Vulkan prompt-processing speedup over best-CPU is 7.27".into(),
            evidence_class: EvidenceClass::DerivedDeterministically,
            source_artifact_sha256: Some(art().sha256_hex),
            source_locator: Some("§1".into()),
            observation_identities: vec![vk.clone(), cpu.clone()],
            harness_run_id: None,
        },
        occurred_at: None,
        recorded_at: None,
        derivation: Some(Derivation::Ratio {
            numerator: vk,
            denominator: cpu,
            decimals: 2,
        }),
    };
    assert!(
        s.put_memory(WriteChannel::Operator, &w).is_ok(),
        "287.17 / 39.52 = 7.2665, which is 7.27 at two decimals"
    );
}

#[test]
fn a_derivation_that_does_not_recompute_is_refused() {
    let (s, vk, cpu) = seeded();
    let w = MemoryWrite {
        terms: MemoryRecordTerms {
            // Plausible, wrong, and the sort of number that survives review.
            claim: "Vulkan prompt-processing speedup over best-CPU is 8.10".into(),
            evidence_class: EvidenceClass::DerivedDeterministically,
            source_artifact_sha256: None,
            source_locator: None,
            observation_identities: vec![vk.clone(), cpu.clone()],
            harness_run_id: None,
        },
        occurred_at: None,
        recorded_at: None,
        derivation: Some(Derivation::Ratio {
            numerator: vk,
            denominator: cpu,
            decimals: 2,
        }),
    };
    match s.put_memory(WriteChannel::Operator, &w) {
        Err(WriteError::Evidence(EvidenceRefusal::DerivationFailed(
            DerivationError::Mismatch {
                claimed,
                recomputed,
            },
        ))) => {
            assert_eq!(claimed, "8.10");
            assert_eq!(recomputed, "7.27");
        }
        other => panic!("expected a mismatch, got {other:?}"),
    }
}

#[test]
fn derived_without_a_named_transform_is_refused() {
    let (s, vk, cpu) = seeded();
    let w = MemoryWrite {
        terms: MemoryRecordTerms {
            claim: "speedup is 7.27".into(),
            evidence_class: EvidenceClass::DerivedDeterministically,
            source_artifact_sha256: None,
            source_locator: None,
            observation_identities: vec![vk, cpu],
            harness_run_id: None,
        },
        occurred_at: None,
        recorded_at: None,
        derivation: None,
    };
    assert!(matches!(
        s.put_memory(WriteChannel::Operator, &w),
        Err(WriteError::Evidence(
            EvidenceRefusal::DerivedNeedsDerivation
        ))
    ));
}

#[test]
fn a_derivation_over_an_unknown_input_cannot_be_checked_and_is_refused() {
    let (s, vk, _) = seeded();
    let w = MemoryWrite {
        terms: MemoryRecordTerms {
            claim: "speedup is 7.27".into(),
            evidence_class: EvidenceClass::DerivedDeterministically,
            source_artifact_sha256: None,
            source_locator: None,
            observation_identities: vec![vk.clone()],
            harness_run_id: None,
        },
        occurred_at: None,
        recorded_at: None,
        derivation: Some(Derivation::Ratio {
            numerator: vk,
            denominator: "dd".repeat(32),
            decimals: 2,
        }),
    };
    assert!(matches!(
        s.put_memory(WriteChannel::Operator, &w),
        Err(WriteError::Evidence(EvidenceRefusal::DerivationFailed(
            DerivationError::UnknownInput { .. }
        )))
    ));
}

#[test]
fn the_smt_penalty_forces_its_baseline_to_be_named() {
    // §3 reports "Generation collapses at 24 threads: -27.4% (Qwen3-8B)" over a
    // table with tg128 = 11.55 at 8 threads (marked "best tg"), 11.45 at 12, and
    // 8.38 at 24. The prose never says which baseline the -27.4% is against.
    //
    //   vs 11.55 (best tg, t=8):  -27.4%   <- what the doc means
    //   vs 11.45 (t=12):          -26.8%
    //
    // Neither is wrong; the ambiguity is. A percentage is a relative quantity,
    // and a relative quantity with an unnamed baseline is the same defect as a
    // divergence with an unnamed reference. Here the baseline is an input
    // digest, so it cannot go unstated.
    let s = Store::open_in_memory().unwrap();
    let (suite_id, _) = s.put_evaluation_suite(&suite()).unwrap();
    let (pol_id, _) = s
        .put_measurement_policy(&policy("genTokensPerSecond"))
        .unwrap();
    let mk = |v: &str, threads: u32| ObservationTerms {
        observation_kind: format!("tg128.t{threads}"),
        quantity_kind: QuantityKind::Absolute,
        value_text: v.into(),
        measurement_policy_identity: pol_id.clone(),
        evaluation_suite_identity: suite_id.clone(),
        reference_execution_identity: None,
        runtime_identity: "llama.cpp-b10188-d0bfb1981".into(),
        artifact_sha256: None,
    };
    let (t24, _) = s.put_observation(&mk("8.38", 24), NOW).unwrap();
    let (t12, _) = s.put_observation(&mk("11.45", 12), NOW).unwrap();
    let (t8, _) = s.put_observation(&mk("11.55", 8), NOW).unwrap();

    let claim_against = |value: &str, baseline: &str, text: &str| MemoryWrite {
        terms: MemoryRecordTerms {
            claim: text.into(),
            evidence_class: EvidenceClass::DerivedDeterministically,
            source_artifact_sha256: None,
            source_locator: Some("§3".into()),
            observation_identities: vec![value.to_string(), baseline.to_string()],
            harness_run_id: None,
        },
        occurred_at: None,
        recorded_at: None,
        derivation: Some(Derivation::PercentChange {
            value: value.to_string(),
            baseline: baseline.to_string(),
            decimals: 1,
        }),
    };

    // The doc's figure is checkable once its baseline is stated.
    assert!(s
        .put_memory(
            WriteChannel::Operator,
            &claim_against(
                &t24,
                &t8,
                "Generation at 24 threads vs best-tg changes by -27.4"
            )
        )
        .is_ok());

    // The other baseline gives a different, equally valid, different number.
    assert!(s
        .put_memory(
            WriteChannel::Operator,
            &claim_against(
                &t24,
                &t12,
                "Generation at 24 threads vs 12 changes by -26.8"
            )
        )
        .is_ok());

    // Polarity: pairing the doc's headline number with the WRONG baseline is
    // refused. That mistake is invisible in prose and mechanical here.
    match s.put_memory(
        WriteChannel::Operator,
        &claim_against(&t24, &t12, "Generation at 24 threads changes by -27.4"),
    ) {
        Err(WriteError::Evidence(EvidenceRefusal::DerivationFailed(
            DerivationError::Mismatch {
                claimed,
                recomputed,
            },
        ))) => {
            assert_eq!(claimed, "-27.4");
            assert_eq!(recomputed, "-26.8");
        }
        other => panic!("expected the baseline mismatch to be caught, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Supersession
// ---------------------------------------------------------------------------

#[test]
fn supersession_retires_without_deleting_and_records_the_edge() {
    let (s, vk, _) = seeded();
    let mk = |claim: &str| MemoryWrite {
        terms: MemoryRecordTerms {
            claim: claim.into(),
            evidence_class: EvidenceClass::ExternalClaim,
            source_artifact_sha256: None,
            source_locator: None,
            observation_identities: vec![vk.clone()],
            harness_run_id: None,
        },
        occurred_at: None,
        recorded_at: None,
        derivation: None,
    };
    let (old, _) = s
        .put_memory(WriteChannel::Operator, &mk("14B pp512 is 159.96 (battery)"))
        .unwrap();
    let (new, _) = s
        .put_memory(WriteChannel::Operator, &mk("14B pp512 is 147.91 (AC)"))
        .unwrap();

    s.supersede(&old, &new, NOW).unwrap();

    // Still present. Retirement is not deletion -- the evidence that the belief
    // changed is itself evidence.
    let (superseded_by, at): (Option<String>, Option<String>) = s
        .conn
        .query_row(
            "SELECT superseded_by, superseded_at FROM memories WHERE record_digest = ?1",
            [&old],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(superseded_by.as_deref(), Some(new.as_str()));
    assert_eq!(at.as_deref(), Some(NOW));

    // The claim text is untouched: superseding must not rewrite what was said.
    let claim: String = s
        .conn
        .query_row(
            "SELECT claim FROM memories WHERE record_digest = ?1",
            [&old],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(claim, "14B pp512 is 159.96 (battery)");

    let edge: i64 = s
        .conn
        .query_row(
            "SELECT count(*) FROM provenance_edges
             WHERE src_digest = ?1 AND dst_digest = ?2 AND edge_kind = 'supersedes'",
            [&new, &old],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(edge, 1);
}

#[test]
fn an_invalid_observation_is_refused_before_it_reaches_sql() {
    let (s, _, _) = seeded();
    let orphan = ObservationTerms {
        observation_kind: "maxLogitDivergence".into(),
        quantity_kind: QuantityKind::Relative,
        value_text: "4.3362".into(),
        measurement_policy_identity: "ee".repeat(32),
        evaluation_suite_identity: "ff".repeat(32),
        reference_execution_identity: None, // relative with no referent
        runtime_identity: "llama.cpp".into(),
        artifact_sha256: None,
    };
    match s.put_observation(&orphan, NOW) {
        Err(WriteError::Observation(d)) => {
            assert!(d.contains(&ObservationDefect::RelativeWithoutReference));
        }
        other => panic!("expected the pure layer to catch it first, got {other:?}"),
    }
}
