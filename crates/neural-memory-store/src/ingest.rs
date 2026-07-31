//! Declarative evidence ingestion. **Operator channel.**
//!
//! Until now evidence could only be added by writing a Rust module and
//! recompiling. That is tenable for two frozen corpora and untenable for
//! anything ongoing, and it was the single thing standing between "works" and
//! "usable".
//!
//! An ingest document is TOML the operator writes by hand. TOML rather than
//! JSON because evidence needs comments — *where* a figure came from is often
//! more important than the figure, and a format that cannot record it in place
//! invites that context to be lost.
//!
//! ## What the format does not let you do
//!
//! The doctrine is preserved by construction, not by review:
//!
//! - **A relative quantity must name a reference.** The observation is rejected
//!   at parse time, before any database sees it, with the same message the
//!   store gives — a divergence with nothing to diverge from is not a
//!   measurement.
//! - **Evidence class is still derived.** Declaring `evidence = "observed"`
//!   does not make it so; the artifact must exist. `evidence = "derived"`
//!   requires a transform whose arithmetic the store re-runs and compares.
//! - **Local aliases, not digests.** You write `observations = ["t24"]`, and
//!   the importer resolves it. Hand-copying 64-hex digests is how corpora
//!   acquire silent mis-wirings.
//! - **Idempotent.** Re-running an unchanged document writes nothing and
//!   advances no history, because identity is still the content seal.
//!
//! Everything here runs through the same `WriteChannel::Operator` path as the
//! hand-written importers, so nothing about the trust model changes — this is a
//! different way to author the same writes, not a new door into the store.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::derive::Derivation;
use crate::write::{MemoryWrite, WriteChannel};
use crate::Store;
use neural_memory_domain::{
    ArtifactTerms, EvaluationSuiteTerms, EvidenceClass, MeasurementPolicyTerms, MemoryRecordTerms,
    ObservationTerms, QuantityKind, ReferenceExecutionTerms,
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IngestDoc {
    pub version: u32,
    pub run_id: Option<String>,
    pub recorded_at: String,
    #[serde(default)]
    pub artifact: Vec<Artifact>,
    #[serde(default)]
    pub suite: Vec<Suite>,
    #[serde(default)]
    pub policy: Vec<Policy>,
    #[serde(default, rename = "reference")]
    pub reference: Vec<Reference>,
    #[serde(default)]
    pub observation: Vec<Observation>,
    #[serde(default)]
    pub claim: Vec<Claim>,
    #[serde(default)]
    pub edge: Vec<Edge>,
    #[serde(default)]
    pub supersede: Vec<Supersede>,
    #[serde(default)]
    pub entity: Vec<Entity>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entity {
    pub name: String,
    #[serde(rename = "type")]
    pub entity_type: String,
    /// Surface forms that resolve to this entity. The point of the branch is
    /// aliases that share no wording with the records -- an alias that merely
    /// repeats the name adds nothing lexical search does not already do.
    #[serde(default)]
    pub aliases: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
    pub id: String,
    pub kind: String,
    pub sha256: String,
    pub bytes: u64,
    pub media_type: String,
    pub uri: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Suite {
    pub id: String,
    pub name: String,
    /// Case digests. Use `case_texts` instead to have them hashed for you.
    #[serde(default)]
    pub cases: Vec<String>,
    #[serde(default)]
    pub case_texts: Vec<String>,
    pub tokenizer: String,
    pub context_cap: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub id: String,
    pub metric: String,
    pub aggregation: String,
    pub comparison_rule: String,
    pub step_budget: Option<u32>,
    pub unit: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reference {
    pub id: String,
    pub runtime: String,
    pub backend: String,
    pub artifact: String,
    pub suite: String,
    #[serde(default)]
    pub environment: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub id: String,
    pub kind: String,
    /// `absolute` or `relative`. A relative quantity requires `reference`.
    pub quantity: String,
    /// Canonical decimal TEXT, quoted. A bare TOML float would be reformatted
    /// by the serializer and the seal would stop being reproducible.
    pub value: toml::Value,
    pub policy: String,
    pub suite: String,
    pub runtime: String,
    pub reference: Option<String>,
    pub artifact: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Claim {
    pub id: String,
    pub text: String,
    /// `observed` | `derived` | `decision` | `inference` | `external`
    pub evidence: String,
    pub artifact: Option<String>,
    pub locator: Option<String>,
    #[serde(default)]
    pub observations: Vec<String>,
    pub occurred_at: Option<String>,
    pub derivation: Option<DerivationSpec>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DerivationSpec {
    /// `ratio` | `delta` | `percentChange`
    pub transform: String,
    pub decimals: u32,
    pub numerator: Option<String>,
    pub denominator: Option<String>,
    pub minuend: Option<String>,
    pub subtrahend: Option<String>,
    pub value: Option<String>,
    pub baseline: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub kind: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Supersede {
    pub retired: String,
    pub replacement: String,
}

#[derive(Debug)]
pub struct IngestReport {
    pub inserted: usize,
    pub already_present: usize,
    pub claims: usize,
    pub observations: usize,
    pub edges: usize,
    pub mentions: usize,
    pub dry_run: bool,
}

impl std::fmt::Display for IngestReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}{} inserted, {} already present ({} claims, {} observations, {} edges, {} mentions)",
            if self.dry_run {
                "DRY RUN: would be "
            } else {
                ""
            },
            self.inserted,
            self.already_present,
            self.claims,
            self.observations,
            self.edges,
            self.mentions
        )
    }
}

fn alias<'a>(
    map: &'a BTreeMap<String, String>,
    kind: &str,
    id: &str,
) -> Result<&'a String, String> {
    map.get(id).ok_or_else(|| {
        let known: Vec<&str> = map.keys().map(String::as_str).collect();
        format!("unknown {kind} alias {id:?}; defined: {known:?}")
    })
}

/// Parse and validate without touching the database.
///
/// Separate from `ingest` so an operator can check a document before it becomes
/// history. Evidence you have to delete afterwards was never append-only.
pub fn validate(text: &str) -> Result<IngestDoc, String> {
    let doc: IngestDoc = toml::from_str(text).map_err(|e| e.to_string())?;
    if doc.version != 1 {
        return Err(format!("unsupported version {}, expected 1", doc.version));
    }
    for o in &doc.observation {
        if !o.value.is_str() {
            return Err(format!(
                "observation {:?}: value must be a QUOTED decimal string, e.g. value = \"8.38\". \
                 A bare TOML number is a float, and a float in a sealed document is at the \
                 mercy of the serialiser's formatter.",
                o.id
            ));
        }
        match o.quantity.as_str() {
            "relative" if o.reference.is_none() => {
                return Err(format!(
                    "observation {:?} is relative but names no reference. A divergence with \
                     nothing to diverge from is not a measurement.",
                    o.id
                ))
            }
            "absolute" if o.reference.is_some() => {
                return Err(format!(
                    "observation {:?} is absolute but names a reference; that field is out of \
                     scope for it and would be silently ignored.",
                    o.id
                ))
            }
            "relative" | "absolute" => {}
            other => {
                return Err(format!(
                    "observation {:?}: unknown quantity {other:?}",
                    o.id
                ))
            }
        }
    }
    for c in &doc.claim {
        match c.evidence.as_str() {
            "derived" if c.derivation.is_none() => {
                return Err(format!(
                    "claim {:?} declares evidence = \"derived\" but names no transform, so \
                     nothing can recompute it.",
                    c.id
                ))
            }
            "observed" if c.artifact.is_none() => {
                return Err(format!(
                    "claim {:?} declares evidence = \"observed\" but cites no artifact. A claim \
                     with no artifact is an inference about the world, not an observation of it.",
                    c.id
                ))
            }
            "observed" | "derived" | "decision" | "inference" | "external" => {}
            other => {
                return Err(format!(
                    "claim {:?}: unknown evidence class {other:?}",
                    c.id
                ))
            }
        }
    }
    Ok(doc)
}

/// Apply a validated document. `dry_run` parses, resolves and checks every
/// reference without writing.
pub fn ingest(store: &Store, text: &str, dry_run: bool) -> Result<IngestReport, String> {
    let doc = validate(text)?;
    let at = doc.recorded_at.clone();
    let mut rep = IngestReport {
        inserted: 0,
        already_present: 0,
        claims: 0,
        observations: 0,
        edges: 0,
        mentions: 0,
        dry_run,
    };

    // A dry run executes against the REAL store inside a transaction that is
    // then rolled back. Running it against a scratch database instead would
    // report every already-present record as an insert, so the one number the
    // operator is checking -- how much this document actually adds -- would be
    // wrong exactly when it matters.
    let rollback = if dry_run {
        Some(
            store
                .conn
                .unchecked_transaction()
                .map_err(|e| format!("dry run: {e}"))?,
        )
    } else {
        None
    };
    let target: &Store = store;

    let mut artifacts = BTreeMap::new();
    for a in &doc.artifact {
        let (_, w) = target
            .put_artifact(
                &ArtifactTerms {
                    artifact_kind: a.kind.clone(),
                    sha256_hex: a.sha256.clone(),
                    byte_size: a.bytes,
                    media_type: a.media_type.clone(),
                    source_uri: a.uri.clone(),
                },
                &at,
            )
            .map_err(|e| format!("artifact {:?}: {e}", a.id))?;
        if w.inserted() {
            rep.inserted += 1;
        } else {
            rep.already_present += 1;
        }
        artifacts.insert(a.id.clone(), a.sha256.clone());
    }

    let mut suites = BTreeMap::new();
    for s in &doc.suite {
        let mut cases = s.cases.clone();
        cases.extend(
            s.case_texts
                .iter()
                .map(|t| neural_memory_domain::sha256_hex(t.as_bytes())),
        );
        let (id, _) = target
            .put_evaluation_suite(&EvaluationSuiteTerms {
                suite_name: s.name.clone(),
                case_digests: cases,
                tokenizer_identity: s.tokenizer.clone(),
                context_cap: s.context_cap,
            })
            .map_err(|e| format!("suite {:?}: {e}", s.id))?;
        suites.insert(s.id.clone(), id);
    }

    let mut policies = BTreeMap::new();
    for p in &doc.policy {
        let (id, _) = target
            .put_measurement_policy(&MeasurementPolicyTerms {
                metric: p.metric.clone(),
                aggregation: p.aggregation.clone(),
                comparison_rule: p.comparison_rule.clone(),
                step_budget: p.step_budget,
                unit: p.unit.clone(),
            })
            .map_err(|e| format!("policy {:?}: {e}", p.id))?;
        policies.insert(p.id.clone(), id);
    }

    let mut references = BTreeMap::new();
    for r in &doc.reference {
        let (id, _) = target
            .put_reference_execution(&ReferenceExecutionTerms {
                runtime_identity: r.runtime.clone(),
                backend_id: r.backend.clone(),
                artifact_sha256: alias(&artifacts, "artifact", &r.artifact)?.clone(),
                evaluation_suite_identity: alias(&suites, "suite", &r.suite)?.clone(),
                environment: r.environment.clone(),
            })
            .map_err(|e| format!("reference {:?}: {e}", r.id))?;
        references.insert(r.id.clone(), id);
    }

    let mut observations = BTreeMap::new();
    for o in &doc.observation {
        let terms = ObservationTerms {
            observation_kind: o.kind.clone(),
            quantity_kind: if o.quantity == "relative" {
                QuantityKind::Relative
            } else {
                QuantityKind::Absolute
            },
            value_text: o.value.as_str().expect("validated").to_string(),
            measurement_policy_identity: alias(&policies, "policy", &o.policy)?.clone(),
            evaluation_suite_identity: alias(&suites, "suite", &o.suite)?.clone(),
            reference_execution_identity: match &o.reference {
                Some(r) => Some(alias(&references, "reference", r)?.clone()),
                None => None,
            },
            runtime_identity: o.runtime.clone(),
            artifact_sha256: match &o.artifact {
                Some(a) => Some(alias(&artifacts, "artifact", a)?.clone()),
                None => None,
            },
        };
        let (id, w) = target
            .put_observation(&terms, &at)
            .map_err(|e| format!("observation {:?}: {e}", o.id))?;
        if w.inserted() {
            rep.inserted += 1;
        } else {
            rep.already_present += 1;
        }
        rep.observations += 1;
        observations.insert(o.id.clone(), id);
    }

    let mut claims = BTreeMap::new();
    for c in &doc.claim {
        let obs: Result<Vec<String>, String> = c
            .observations
            .iter()
            .map(|o| alias(&observations, "observation", o).cloned())
            .collect();
        let derivation = match &c.derivation {
            None => None,
            Some(d) => Some(build_derivation(d, &observations)?),
        };
        let w = MemoryWrite {
            terms: MemoryRecordTerms {
                claim: c.text.clone(),
                evidence_class: match c.evidence.as_str() {
                    "observed" => EvidenceClass::Observed,
                    "derived" => EvidenceClass::DerivedDeterministically,
                    "decision" => EvidenceClass::HumanDecision,
                    "inference" => EvidenceClass::AgentInference,
                    _ => EvidenceClass::ExternalClaim,
                },
                source_artifact_sha256: match &c.artifact {
                    Some(a) => Some(alias(&artifacts, "artifact", a)?.clone()),
                    None => None,
                },
                source_locator: c.locator.clone(),
                observation_identities: obs?,
                harness_run_id: doc.run_id.clone(),
            },
            occurred_at: c.occurred_at.as_deref().or(Some(&at)),
            derivation,
        };
        let (id, wrote) = target
            .put_memory(WriteChannel::Operator, &w)
            .map_err(|e| format!("claim {:?}: {e}", c.id))?;
        if wrote.inserted() {
            rep.inserted += 1;
        } else {
            rep.already_present += 1;
        }
        rep.claims += 1;
        claims.insert(c.id.clone(), id);
    }

    // Entities first, then a reindex, so mentions reflect the dictionary this
    // document declares rather than whatever was there before.
    if !doc.entity.is_empty() {
        for e in &doc.entity {
            target
                .put_entity(&neural_memory_domain::EntityTerms {
                    canonical_name: e.name.clone(),
                    entity_type: e.entity_type.clone(),
                    aliases: e.aliases.clone(),
                })
                .map_err(|err| format!("entity {:?}: {err}", e.name))?;
        }
    }

    for e in &doc.edge {
        target
            .add_edge(
                alias(&claims, "claim", &e.from)?,
                alias(&claims, "claim", &e.to)?,
                &e.kind,
                &at,
            )
            .map_err(|e2| format!("edge {} -> {}: {e2}", e.from, e.to))?;
        rep.edges += 1;
    }
    for s in &doc.supersede {
        target
            .supersede(
                alias(&claims, "claim", &s.retired)?,
                alias(&claims, "claim", &s.replacement)?,
                &at,
            )
            .map_err(|e| format!("supersede {} -> {}: {e}", s.retired, s.replacement))?;
    }
    // Reindex whenever entities exist, not only when this document declared
    // some: a document adding claims to a store that already has a dictionary
    // must have those claims indexed too, or they are invisible to the branch.
    let (dict, _) = target.entity_dictionary().map_err(|e| e.to_string())?;
    if !dict.is_empty() {
        let (recs, mentions) = target.reindex_mentions().map_err(|e| e.to_string())?;
        rep.mentions = mentions;
        let _ = recs;
    }

    if let Some(tx) = rollback {
        tx.rollback()
            .map_err(|e| format!("dry run rollback: {e}"))?;
    }
    Ok(rep)
}

fn build_derivation(
    d: &DerivationSpec,
    obs: &BTreeMap<String, String>,
) -> Result<Derivation, String> {
    let need = |field: &str, v: &Option<String>| -> Result<String, String> {
        let id = v
            .as_ref()
            .ok_or_else(|| format!("transform {:?} requires {field}", d.transform))?;
        alias(obs, "observation", id).cloned()
    };
    Ok(match d.transform.as_str() {
        "ratio" => Derivation::Ratio {
            numerator: need("numerator", &d.numerator)?,
            denominator: need("denominator", &d.denominator)?,
            decimals: d.decimals,
        },
        "delta" => Derivation::Delta {
            minuend: need("minuend", &d.minuend)?,
            subtrahend: need("subtrahend", &d.subtrahend)?,
            decimals: d.decimals,
        },
        "percentChange" => Derivation::PercentChange {
            value: need("value", &d.value)?,
            baseline: need("baseline", &d.baseline)?,
            decimals: d.decimals,
        },
        other => {
            return Err(format!(
                "unknown transform {other:?}; known: ratio, delta, percentChange"
            ))
        }
    })
}
