//! Sort discipline and seal properties.
//!
//! Every list field is either SORTED or ORDER SIGNIFICANT, and both polarities
//! are asserted: reordering a SORTED list must NOT change the digest, reordering
//! an ORDER SIGNIFICANT one MUST. A test that only checks one direction passes
//! against a digest function that ignores the field entirely.

use neural_memory_domain::*;

fn suite(cases: &[&str]) -> EvaluationSuiteTerms {
    EvaluationSuiteTerms {
        suite_name: "gpd-logit-divergence".into(),
        case_digests: cases.iter().map(|s| s.to_string()).collect(),
        tokenizer_identity: "qwen3-tokenizer-v1".into(),
        context_cap: 8192,
    }
}

const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const C: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

#[test]
fn evaluation_suite_case_order_is_not_significant() {
    let forward = evaluation_suite_identity(&suite(&[A, B, C]));
    let shuffled = evaluation_suite_identity(&suite(&[C, A, B]));
    assert_eq!(
        forward, shuffled,
        "case_digests is SORTED: a suite is a set, and listing order must not fork its identity"
    );

    // Polarity: membership DOES matter. If it did not, the digest would be
    // ignoring the field and the test above would pass vacuously.
    let different = evaluation_suite_identity(&suite(&[A, B]));
    assert_ne!(forward, different, "dropping a case must change the suite");
}

#[test]
fn session_prefix_order_is_significant() {
    let forward = session_prefix_identity(&SessionPrefix {
        emitted: vec![A.into(), B.into(), C.into()],
    });
    let reordered = session_prefix_identity(&SessionPrefix {
        emitted: vec![C.into(), A.into(), B.into()],
    });
    assert_ne!(
        forward, reordered,
        "emitted is ORDER SIGNIFICANT: it is the byte order the model saw, and \
         reordering it is precisely the cache invalidation the type prevents"
    );

    // Polarity: the same order really is the same identity.
    let same = session_prefix_identity(&SessionPrefix {
        emitted: vec![A.into(), B.into(), C.into()],
    });
    assert_eq!(forward, same);
}

#[test]
fn memory_record_observation_order_is_not_significant() {
    let mk = |obs: Vec<String>| MemoryRecordTerms {
        claim: "Vulkan exceeded the preregistered tolerance".into(),
        evidence_class: EvidenceClass::DerivedDeterministically,
        source_artifact_sha256: Some(A.into()),
        source_locator: Some("§4 table".into()),
        observation_identities: obs,
        harness_run_id: None,
    };
    assert_eq!(
        memory_record_identity(&mk(vec![A.into(), B.into()])),
        memory_record_identity(&mk(vec![B.into(), A.into()])),
        "observation_identities is SORTED: a set of supports"
    );
    assert_ne!(
        memory_record_identity(&mk(vec![A.into(), B.into()])),
        memory_record_identity(&mk(vec![A.into()])),
        "polarity: which observations support a claim must change its identity"
    );
}

#[test]
fn reference_execution_environment_order_is_not_significant() {
    let mk = |env: Vec<&str>| ReferenceExecutionTerms {
        runtime_identity: "llama.cpp-b10188-d0bfb1981".into(),
        backend_id: "llama-cpp-cpu".into(),
        artifact_sha256: A.into(),
        evaluation_suite_identity: B.into(),
        environment: env.iter().map(|s| s.to_string()).collect(),
    };
    assert_eq!(
        reference_execution_identity(&mk(vec!["os=ubuntu-26.04", "governor=performance"])),
        reference_execution_identity(&mk(vec!["governor=performance", "os=ubuntu-26.04"])),
    );
    assert_ne!(
        reference_execution_identity(&mk(vec!["os=ubuntu-26.04"])),
        reference_execution_identity(&mk(vec!["os=ubuntu-24.04"])),
        "polarity: the environment is part of what makes a reference a reference"
    );
}

#[test]
fn every_measurement_policy_term_is_substantive() {
    let base = MeasurementPolicyTerms {
        metric: "maxAbsoluteLogitDelta".into(),
        aggregation: "maxOverSteps".into(),
        comparison_rule: "lessThanOrEqualTolerance".into(),
        step_budget: Some(32),
        unit: "logit".into(),
    };
    let id = measurement_policy_identity(&base);

    // Changing ANY term must change the seal. If `metric` did not participate,
    // two policies measuring different quantities would share one identity --
    // which is the exact defect this type exists to fix.
    let mutations: Vec<(&str, MeasurementPolicyTerms)> = vec![
        (
            "metric",
            MeasurementPolicyTerms {
                metric: "cosineSimilarity".into(),
                ..base.clone()
            },
        ),
        (
            "aggregation",
            MeasurementPolicyTerms {
                aggregation: "meanOverSteps".into(),
                ..base.clone()
            },
        ),
        (
            "comparisonRule",
            MeasurementPolicyTerms {
                comparison_rule: "strictlyLess".into(),
                ..base.clone()
            },
        ),
        (
            "stepBudget",
            MeasurementPolicyTerms {
                step_budget: Some(64),
                ..base.clone()
            },
        ),
        (
            "stepBudgetNone",
            MeasurementPolicyTerms {
                step_budget: None,
                ..base.clone()
            },
        ),
        (
            "unit",
            MeasurementPolicyTerms {
                unit: "nat".into(),
                ..base.clone()
            },
        ),
    ];
    for (name, m) in mutations {
        assert_ne!(
            id,
            measurement_policy_identity(&m),
            "{name} must change the measurement-policy seal"
        );
    }

    // Polarity: an identical policy seals identically.
    assert_eq!(id, measurement_policy_identity(&base.clone()));
}

#[test]
fn identities_are_domain_separated() {
    // Two different record kinds with structurally identical payloads must not
    // collide. Without the domain prefix they could.
    let art = artifact_identity(&ArtifactTerms {
        artifact_kind: "x".into(),
        sha256_hex: A.into(),
        byte_size: 1,
        media_type: "text/plain".into(),
        source_uri: "file:///x".into(),
    });
    let pol = measurement_policy_identity(&MeasurementPolicyTerms {
        metric: "x".into(),
        aggregation: "x".into(),
        comparison_rule: "x".into(),
        step_budget: Some(1),
        unit: "x".into(),
    });
    assert_ne!(art, pol);
    assert!(valid_sha256(&art) && valid_sha256(&pol));
}

#[test]
fn an_identity_is_never_an_input_to_itself() {
    // The seal doctrine: identity derives from substantive TERMS. None of the
    // Terms types carries an id field at all, so self-reference is structurally
    // impossible rather than merely avoided. This test documents that as an
    // executable claim: adding such a field would fail to compile here.
    let terms = ArtifactTerms {
        artifact_kind: "characterization-doc".into(),
        sha256_hex: A.into(),
        byte_size: 25061,
        media_type: "text/markdown".into(),
        source_uri: "file:///home/aurascoper/Downloads/gpd.md".into(),
    };
    let once = artifact_identity(&terms);
    let twice = artifact_identity(&terms);
    assert_eq!(once, twice, "sealing is deterministic");
    assert!(valid_sha256(&once));
}
