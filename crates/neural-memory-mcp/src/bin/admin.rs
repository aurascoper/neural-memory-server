//! Operator CLI. **This is the channel no MCP tool can reach.**
//!
//! That is not a convention — it is what makes `Observed` mean anything. If an
//! agent could ingest an artifact or record an observation, then "backed by an
//! artifact ingested out of band" would be satisfiable by the agent itself, and
//! the evidence ladder would collapse into a single rung with extra steps.
//!
//! Commands:
//!   record-artifact   --db P --sha256 H --kind K --bytes N --media M --uri U --at T
//!   record-observation --db P --kind K --quantity absolute|relative --value V
//!                      --metric M --aggregation A --unit U [--steps N]
//!                      --suite NAME --case-digest H --tokenizer T --context-cap N
//!                      [--reference-backend B --reference-artifact H] --runtime R --at T
//!   record-decision   --db P --claim C [--locator L] --at T
//!   supersede         --db P --retired H --replacement H --at T
//!   stats             --db P
//!   backup            --db P --to B [--no-verify]
//!   verify-backup     --db P --of B

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;

use neural_memory_domain::*;
use neural_memory_store::*;

fn arg(m: &HashMap<String, String>, k: &str) -> Result<String, String> {
    m.get(k)
        .cloned()
        .ok_or_else(|| format!("--{k} is required"))
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.is_empty() {
        eprintln!("usage: neural-memory-admin <command> --db <path> [options]");
        eprintln!(
            "commands: record-artifact record-observation record-decision supersede stats \
             backup verify-backup"
        );
        return ExitCode::from(2);
    }
    let cmd = argv[0].clone();
    let mut m: HashMap<String, String> = HashMap::new();
    let mut i = 1;
    while i < argv.len() {
        if let Some(k) = argv[i].strip_prefix("--") {
            let v = argv.get(i + 1).cloned().unwrap_or_default();
            m.insert(k.to_string(), v);
            i += 2;
        } else {
            eprintln!("unexpected argument: {}", argv[i]);
            return ExitCode::from(2);
        }
    }

    match run(&cmd, &m) {
        Ok(msg) => {
            println!("{msg}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(1)
        }
    }
}

fn run(cmd: &str, m: &HashMap<String, String>) -> Result<String, String> {
    let db = PathBuf::from(arg(m, "db")?);
    let store = Store::open(&db).map_err(|e| e.to_string())?;

    match cmd {
        "record-artifact" => {
            let a = ArtifactTerms {
                artifact_kind: arg(m, "kind")?,
                sha256_hex: arg(m, "sha256")?,
                byte_size: arg(m, "bytes")?
                    .parse()
                    .map_err(|_| "--bytes must be an integer")?,
                media_type: arg(m, "media")?,
                source_uri: arg(m, "uri")?,
            };
            if !valid_sha256(&a.sha256_hex) {
                return Err("--sha256 must be 64 lowercase hex characters".into());
            }
            let (id, w) = store
                .put_artifact(&a, &arg(m, "at")?)
                .map_err(|e| e.to_string())?;
            Ok(format!("artifact {} identity={id}", verb(w)))
        }

        "record-observation" => {
            let suite = EvaluationSuiteTerms {
                suite_name: arg(m, "suite")?,
                case_digests: vec![arg(m, "case-digest")?],
                tokenizer_identity: arg(m, "tokenizer")?,
                context_cap: arg(m, "context-cap")?
                    .parse()
                    .map_err(|_| "--context-cap must be an integer")?,
            };
            let (suite_id, _) = store
                .put_evaluation_suite(&suite)
                .map_err(|e| e.to_string())?;

            let policy = MeasurementPolicyTerms {
                metric: arg(m, "metric")?,
                aggregation: arg(m, "aggregation")?,
                comparison_rule: m
                    .get("comparison-rule")
                    .cloned()
                    .unwrap_or_else(|| "reportOnly".into()),
                step_budget: m.get("steps").and_then(|s| s.parse().ok()),
                unit: arg(m, "unit")?,
            };
            let (pol_id, _) = store
                .put_measurement_policy(&policy)
                .map_err(|e| e.to_string())?;

            let quantity = match arg(m, "quantity")?.as_str() {
                "absolute" => QuantityKind::Absolute,
                "relative" => QuantityKind::Relative,
                _ => return Err("--quantity must be 'absolute' or 'relative'".into()),
            };

            // A relative quantity needs a reference execution. Building it here
            // rather than accepting a bare identity means the operator cannot
            // point at something that does not exist.
            let reference = if quantity == QuantityKind::Relative {
                let r = ReferenceExecutionTerms {
                    runtime_identity: arg(m, "runtime")?,
                    backend_id: arg(m, "reference-backend").map_err(|_| {
                        "--reference-backend is required for a relative quantity: a divergence \
                         with no named reference is not a measurement"
                    })?,
                    artifact_sha256: arg(m, "reference-artifact")
                        .map_err(|_| "--reference-artifact is required for a relative quantity")?,
                    evaluation_suite_identity: suite_id.clone(),
                    environment: m
                        .get("environment")
                        .map(|s| s.split(',').map(str::to_string).collect())
                        .unwrap_or_default(),
                };
                let (id, _) = store
                    .put_reference_execution(&r)
                    .map_err(|e| e.to_string())?;
                Some(id)
            } else {
                None
            };

            let o = ObservationTerms {
                observation_kind: arg(m, "kind")?,
                quantity_kind: quantity,
                value_text: arg(m, "value")?,
                measurement_policy_identity: pol_id,
                evaluation_suite_identity: suite_id,
                reference_execution_identity: reference,
                runtime_identity: arg(m, "runtime")?,
                artifact_sha256: m.get("artifact").cloned(),
            };
            let (id, w) = store
                .put_observation(&o, &arg(m, "at")?)
                .map_err(|e| e.to_string())?;
            Ok(format!("observation {} identity={id}", verb(w)))
        }

        "record-decision" => {
            let w = MemoryWrite {
                terms: MemoryRecordTerms {
                    claim: arg(m, "claim")?,
                    evidence_class: EvidenceClass::HumanDecision,
                    source_artifact_sha256: m.get("artifact").cloned(),
                    source_locator: m.get("locator").cloned(),
                    observation_identities: vec![],
                    harness_run_id: None,
                },
                occurred_at: m.get("at").map(String::as_str),
                recorded_at: m.get("at").map(String::as_str),
                derivation: None,
            };
            let (d, wr) = store
                .put_memory(WriteChannel::Operator, &w)
                .map_err(|e| e.to_string())?;
            Ok(format!("decision {} digest={d}", verb(wr)))
        }

        "supersede" => {
            let retired = arg(m, "retired")?;
            let replacement = arg(m, "replacement")?;
            for d in [&retired, &replacement] {
                if store.get_record(d).map_err(|e| e.to_string())?.is_none() {
                    return Err(format!("no such record: {d}"));
                }
            }
            store
                .supersede(&retired, &replacement, &arg(m, "at")?)
                .map_err(|e| e.to_string())?;
            Ok(format!("{retired} retired in favour of {replacement}"))
        }

        "stats" => {
            let count = |t: &str| -> i64 {
                store
                    .conn
                    .query_row(&format!("SELECT count(*) FROM {t}"), [], |r| r.get(0))
                    .unwrap_or(-1)
            };
            Ok(format!(
                "memories={} observations={} artifacts={} edges={} maxSeq={} integrity={}",
                count("memories"),
                count("observations"),
                count("artifacts"),
                count("provenance_edges"),
                store.max_recorded_seq().map_err(|e| e.to_string())?,
                store.integrity_ok().map_err(|e| e.to_string())?
            ))
        }

        // `VACUUM INTO`, not `cp`. In WAL mode a committed transaction lives in
        // `store.db-wal` until a checkpoint, so copying `store.db` alone can
        // omit history that is fully committed — and the copy still opens and
        // still passes `integrity_check`, so the loss is silent.
        //
        // Verification runs by default and its failure is the command's failure.
        // A backup nobody checked is a guess about a file.
        "backup" => {
            let to = PathBuf::from(arg(m, "to")?);
            let report = store.backup_to(&to).map_err(|e| e.to_string())?;

            if m.contains_key("no-verify") {
                return Ok(format!(
                    "backed up to {} ({} bytes, {} records, {} observations, {} edges, schema {}) \
                     -- NOT VERIFIED (--no-verify)",
                    report.destination,
                    report.bytes,
                    report.records,
                    report.observations,
                    report.edges,
                    report.schema_version
                ));
            }

            let diffs = verify_replica(&store, &to).map_err(|e| e.to_string())?;
            if !diffs.is_empty() {
                // Leave the bad copy in place. Deleting the evidence of a failed
                // backup is how the same failure happens again unexplained.
                return Err(format!(
                    "backup wrote {} but verification found {} difference(s); \
                     the file has been left in place for inspection:\n  {}",
                    to.display(),
                    diffs.len(),
                    diffs
                        .iter()
                        .take(10)
                        .map(|d| d.to_string())
                        .collect::<Vec<_>>()
                        .join("\n  ")
                ));
            }
            Ok(format!(
                "backed up to {} ({} bytes, {} records, {} observations, {} edges, schema {}) \
                 -- verified: every record digest, observation identity and provenance edge matches",
                report.destination,
                report.bytes,
                report.records,
                report.observations,
                report.edges,
                report.schema_version
            ))
        }

        // Verify a backup taken earlier, without taking a new one.
        "verify-backup" => {
            let of = PathBuf::from(arg(m, "of")?);
            let diffs = verify_replica(&store, &of).map_err(|e| e.to_string())?;
            if diffs.is_empty() {
                Ok(format!("{} matches {} exactly", of.display(), db.display()))
            } else {
                Err(format!(
                    "{} differs from {} in {} way(s):\n  {}",
                    of.display(),
                    db.display(),
                    diffs.len(),
                    diffs
                        .iter()
                        .take(20)
                        .map(|d| d.to_string())
                        .collect::<Vec<_>>()
                        .join("\n  ")
                ))
            }
        }

        other => Err(format!("unknown command: {other}")),
    }
}

fn verb(w: Wrote) -> &'static str {
    if w.inserted() {
        "recorded"
    } else {
        "already present"
    }
}
