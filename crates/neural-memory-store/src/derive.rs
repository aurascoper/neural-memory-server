//! Deterministic derivations, and the recomputation that makes them checkable.
//!
//! `EvidenceClass::DerivedDeterministically` is the one class where enforcement
//! is cheap and total: if the store cannot recompute the claim from named inputs
//! and get the same answer, the write is rejected. There is no honour system
//! here and no place to assert the class.
//!
//! This exists because the characterization document is full of quantities that
//! *look* like observations and are not. "Vulkan speedup 7.27x" is not measured
//! — it is `287.17 / 39.52`, a ratio of two observations. Recording it as
//! `Observed` would launder a derivation into a measurement. Recording it as
//! `DerivedDeterministically` without naming the transform and its inputs would
//! be the same laundering with a better label.

use serde::{Deserialize, Serialize};

/// A named transform over named inputs. Both are required: a transform without
/// inputs cannot be checked, and inputs without a transform do not say what was
/// done to them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "transform")]
pub enum Derivation {
    /// `numerator / denominator`, e.g. a speedup.
    Ratio {
        numerator: String,
        denominator: String,
        /// Decimals the result is reported to. Comparison is exact at this
        /// precision — a derivation that only matches "about right" is not one.
        decimals: u32,
    },
    /// `minuend - subtrahend`, e.g. a regression.
    Delta {
        minuend: String,
        subtrahend: String,
        decimals: u32,
    },
    /// `(value - baseline) / baseline * 100`, e.g. "−27.4%".
    PercentChange {
        value: String,
        baseline: String,
        decimals: u32,
    },
}

impl Derivation {
    /// Observation identities this derivation reads.
    pub fn inputs(&self) -> Vec<&str> {
        match self {
            Derivation::Ratio {
                numerator,
                denominator,
                ..
            } => vec![numerator, denominator],
            Derivation::Delta {
                minuend,
                subtrahend,
                ..
            } => vec![minuend, subtrahend],
            Derivation::PercentChange {
                value, baseline, ..
            } => vec![value, baseline],
        }
    }

    fn decimals(&self) -> u32 {
        match self {
            Derivation::Ratio { decimals, .. }
            | Derivation::Delta { decimals, .. }
            | Derivation::PercentChange { decimals, .. } => *decimals,
        }
    }

    /// Recompute from resolved input values, in the same order as `inputs()`.
    pub fn recompute(&self, values: &[f64]) -> Result<String, DerivationError> {
        if values.len() != self.inputs().len() {
            return Err(DerivationError::WrongArity {
                expected: self.inputs().len(),
                got: values.len(),
            });
        }
        let raw = match self {
            Derivation::Ratio { .. } => {
                if values[1] == 0.0 {
                    return Err(DerivationError::DivideByZero);
                }
                values[0] / values[1]
            }
            Derivation::Delta { .. } => values[0] - values[1],
            Derivation::PercentChange { .. } => {
                if values[1] == 0.0 {
                    return Err(DerivationError::DivideByZero);
                }
                (values[0] - values[1]) / values[1] * 100.0
            }
        };
        if !raw.is_finite() {
            return Err(DerivationError::NotFinite);
        }
        Ok(format_fixed(raw, self.decimals()))
    }

    /// Does the claimed value match what the inputs actually produce?
    pub fn verify(&self, values: &[f64], claimed: &str) -> Result<(), DerivationError> {
        let recomputed = self.recompute(values)?;
        // Compare at the declared precision, so "7.27" and "7.2700" agree and
        // "7.27" and "7.28" do not.
        let claimed_norm = claimed
            .trim()
            .parse::<f64>()
            .map(|v| format_fixed(v, self.decimals()))
            .map_err(|_| DerivationError::ClaimNotDecimal {
                claimed: claimed.to_string(),
            })?;
        if claimed_norm == recomputed {
            Ok(())
        } else {
            Err(DerivationError::Mismatch {
                claimed: claimed_norm,
                recomputed,
            })
        }
    }
}

fn format_fixed(v: f64, decimals: u32) -> String {
    // Normalise "-0.00" to "0.00": the sign of a zero is not a finding.
    let s = format!("{:.*}", decimals as usize, v);
    if s.chars().all(|c| matches!(c, '-' | '0' | '.')) && s.starts_with('-') {
        s[1..].to_string()
    } else {
        s
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DerivationError {
    WrongArity {
        expected: usize,
        got: usize,
    },
    DivideByZero,
    NotFinite,
    ClaimNotDecimal {
        claimed: String,
    },
    Mismatch {
        claimed: String,
        recomputed: String,
    },
    /// An input observation identity is not in the store, so the derivation
    /// cannot be checked and therefore cannot be accepted.
    UnknownInput {
        identity: String,
    },
    /// An input's value is not a decimal, so it cannot participate.
    InputNotDecimal {
        identity: String,
        value_text: String,
    },
}

impl std::fmt::Display for DerivationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DerivationError::WrongArity { expected, got } => {
                write!(f, "transform takes {expected} inputs, got {got}")
            }
            DerivationError::DivideByZero => write!(f, "divide by zero"),
            DerivationError::NotFinite => write!(f, "result is not finite"),
            DerivationError::ClaimNotDecimal { claimed } => {
                write!(f, "claimed value {claimed:?} is not a decimal")
            }
            DerivationError::Mismatch {
                claimed,
                recomputed,
            } => write!(
                f,
                "claimed {claimed}, but the named inputs recompute to {recomputed}"
            ),
            DerivationError::UnknownInput { identity } => {
                write!(f, "input observation {identity} is not in the store")
            }
            DerivationError::InputNotDecimal {
                identity,
                value_text,
            } => write!(f, "input {identity} has non-decimal value {value_text:?}"),
        }
    }
}
