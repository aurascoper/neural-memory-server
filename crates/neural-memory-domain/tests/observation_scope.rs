//! A relative quantity with no named reference must be unrepresentable.
//!
//! This encodes the defect the GPD Vulkan characterization surfaced: the
//! generation contract compares `max_logit_divergence` against a tolerance with
//! no field naming what it diverged *from*, over how many steps, or how the
//! maximum was aggregated. Two labs could both read `Conformant` against the same
//! seal having measured against different references.
//!
//! Both polarities are asserted, and the scope rule cuts both ways — mirroring
//! `MeasurementOutOfScope` upstream, where an *undeclared* tolerance must not be
//! measured, not merely need not be.

use neural_memory_domain::*;

const REF: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const POL: &str = "2222222222222222222222222222222222222222222222222222222222222222";
const SUITE: &str = "3333333333333333333333333333333333333333333333333333333333333333";

fn relative(reference: Option<&str>) -> ObservationTerms {
    ObservationTerms {
        observation_kind: "maxLogitDivergence".into(),
        quantity_kind: QuantityKind::Relative,
        value_text: "4.3362".into(),
        measurement_policy_identity: POL.into(),
        evaluation_suite_identity: SUITE.into(),
        reference_execution_identity: reference.map(|s| s.to_string()),
        runtime_identity: "llama.cpp-b10188-d0bfb1981".into(),
        artifact_sha256: None,
    }
}

fn absolute(reference: Option<&str>) -> ObservationTerms {
    ObservationTerms {
        observation_kind: "promptProcessingTokensPerSecond".into(),
        quantity_kind: QuantityKind::Absolute,
        value_text: "287.17".into(),
        measurement_policy_identity: POL.into(),
        evaluation_suite_identity: SUITE.into(),
        reference_execution_identity: reference.map(|s| s.to_string()),
        runtime_identity: "llama.cpp-b10188-d0bfb1981".into(),
        artifact_sha256: None,
    }
}

#[test]
fn a_relative_quantity_without_a_reference_is_rejected() {
    let errs = validate_observation(&relative(None));
    assert!(
        errs.contains(&ObservationDefect::RelativeWithoutReference),
        "a divergence with nothing to diverge from is not a measurement: {errs:?}"
    );
}

#[test]
fn a_relative_quantity_with_a_reference_is_accepted() {
    // Polarity. Without this, a validator that rejected everything would pass.
    assert!(validate_observation(&relative(Some(REF))).is_empty());
}

#[test]
fn an_absolute_quantity_may_not_carry_a_reference() {
    // Scope cuts both ways: an out-of-scope field is visible, not silently
    // ignored -- the same rule as MeasurementOutOfScope upstream.
    let errs = validate_observation(&absolute(Some(REF)));
    assert!(
        errs.contains(&ObservationDefect::AbsoluteWithReference),
        "{errs:?}"
    );
}

#[test]
fn an_absolute_quantity_without_a_reference_is_accepted() {
    assert!(validate_observation(&absolute(None)).is_empty());
}

#[test]
fn the_reference_participates_in_the_observations_identity() {
    let other = "4444444444444444444444444444444444444444444444444444444444444444";
    assert_ne!(
        observation_identity(&relative(Some(REF))),
        observation_identity(&relative(Some(other))),
        "the same number measured against a different reference is a different \
         observation; if these collided, `may_share` would be comparing nothing"
    );
}

#[test]
fn the_measurement_policy_participates_too() {
    let mut a = relative(Some(REF));
    let mut b = relative(Some(REF));
    a.measurement_policy_identity = POL.into();
    b.measurement_policy_identity =
        "5555555555555555555555555555555555555555555555555555555555555555".into();
    assert_ne!(
        observation_identity(&a),
        observation_identity(&b),
        "4.3362 by max-abs-delta and 4.3362 by cosine are not the same finding"
    );
}

#[test]
fn a_value_that_is_not_a_decimal_is_rejected() {
    let mut o = relative(Some(REF));
    o.value_text = "about four".into();
    assert!(matches!(
        validate_observation(&o).as_slice(),
        [ObservationDefect::ValueNotDecimal { .. }]
    ));

    // Polarity: scientific notation and negatives are legitimate decimals.
    for good in ["4.3362", "-0.5", "1e-9", "0"] {
        let mut ok = relative(Some(REF));
        ok.value_text = good.into();
        assert!(validate_observation(&ok).is_empty(), "{good} should parse");
    }
}

#[test]
fn a_malformed_digest_is_rejected_in_every_identity_field() {
    for (field, mut o) in [
        ("measurementPolicyIdentity", relative(Some(REF))),
        ("evaluationSuiteIdentity", relative(Some(REF))),
        ("referenceExecutionIdentity", relative(Some(REF))),
    ] {
        match field {
            "measurementPolicyIdentity" => o.measurement_policy_identity = "short".into(),
            "evaluationSuiteIdentity" => o.evaluation_suite_identity = "short".into(),
            _ => o.reference_execution_identity = Some("short".into()),
        }
        let errs = validate_observation(&o);
        assert!(
            errs.iter().any(
                |e| matches!(e, ObservationDefect::MalformedDigest { field: f } if *f == field)
            ),
            "{field} must be checked: {errs:?}"
        );
    }
}

#[test]
fn the_gpd_gemma_divergence_is_storable_only_with_its_referent() {
    // The real record from the characterization, as it must be stored: the
    // 4.3362 figure is meaningless without naming the CPU run it was measured
    // against, the suite it ran over, and what "max" meant.
    let policy = MeasurementPolicyTerms {
        metric: "maxAbsoluteLogitDelta".into(),
        aggregation: "maxOverPreDivergenceSteps".into(),
        comparison_rule: "lessThanOrEqualTolerance".into(),
        step_budget: Some(58),
        unit: "logit".into(),
    };
    let suite = EvaluationSuiteTerms {
        suite_name: "gpd-single-prompt-greedy".into(),
        case_digests: vec![
            "6666666666666666666666666666666666666666666666666666666666666666".into(),
        ],
        tokenizer_identity: "gemma4-tokenizer".into(),
        context_cap: 8192,
    };
    let reference = ReferenceExecutionTerms {
        runtime_identity: "llama.cpp-b10188-d0bfb1981".into(),
        backend_id: "llama-cpp-cpu".into(),
        artifact_sha256: "7777777777777777777777777777777777777777777777777777777777777777".into(),
        evaluation_suite_identity: evaluation_suite_identity(&suite),
        environment: vec!["os=ubuntu-26.04".into(), "governor=performance".into()],
    };

    let obs = ObservationTerms {
        observation_kind: "maxLogitDivergence".into(),
        quantity_kind: QuantityKind::Relative,
        value_text: "4.3362".into(),
        measurement_policy_identity: measurement_policy_identity(&policy),
        evaluation_suite_identity: evaluation_suite_identity(&suite),
        reference_execution_identity: Some(reference_execution_identity(&reference)),
        runtime_identity: "llama.cpp-b10188-d0bfb1981".into(),
        artifact_sha256: None,
    };
    assert!(validate_observation(&obs).is_empty());

    // And stripping the referent -- which is how the finding is written in prose
    // today -- makes it unstorable.
    let mut orphaned = obs.clone();
    orphaned.reference_execution_identity = None;
    assert_eq!(
        validate_observation(&orphaned),
        vec![ObservationDefect::RelativeWithoutReference]
    );
}
