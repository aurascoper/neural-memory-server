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
//!   retract           --db P --digest H --reason R --at T
//!   declare-standing  --db P --digest H --at T
//!   standing          --db P [--as-of T]
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

/// Read-write open, for the commands that write.
/// Whether a command may write. Data, not a per-arm decision.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Access {
    Rw,
    Ro,
}

/// Every dispatchable command and its access. **One list, three jobs**: it gates
/// dispatch in `main`, derives the usage string, and classifies the open.
///
/// The three registration sites were the hazard — banner, usage string, match
/// arm, and only the arm compiler-enforced, so a missed entry left a command
/// that worked and could not be discovered. A const that merely *fed* the usage
/// string would prove `usage ⊆ dispatch`, which is the wrong direction; the
/// hazard is `dispatch ⊄ usage`. Gating dispatch on it instead makes an
/// unregistered arm unreachable, so "works but invisible" stops being a state
/// rather than being tested for.
///
/// The access column exists because moving the store open into each arm turned
/// one wrong-by-default into N per-arm choices with nothing classifying them.
/// `Store::open` carries SQLITE_OPEN_CREATE, so an arm that picks it by mistake
/// answers over an empty store it just created. The choice belongs to the list.
const COMMANDS: &[(&str, Access)] = &[
    ("record-artifact", Access::Rw),
    ("record-observation", Access::Rw),
    ("record-decision", Access::Rw),
    ("supersede", Access::Rw),
    ("retract", Access::Rw),
    ("declare-standing", Access::Rw),
    // The first Ro command, and the reason the access column exists.
    ("standing", Access::Ro),
    // Both only read, and both should be Ro. Left Rw deliberately until that
    // change lands on its own: `open_read_only` does not migrate, so a store
    // older than this build would start erroring where it now silently upgrades.
    // Classified here so the gap is visible in the data while it is open.
    ("stats", Access::Rw),
    ("backup", Access::Rw),
    ("verify-backup", Access::Rw),
];

fn access_of(cmd: &str) -> Option<Access> {
    COMMANDS.iter().find(|(c, _)| *c == cmd).map(|(_, a)| *a)
}

/// Open the store the way this command is classified.
///
/// No arm chooses. An unclassified command cannot open anything, which is the
/// same refusal `main` already made — kept so the function is total rather than
/// relying on a caller that has been gated.
fn open_for(cmd: &str, m: &HashMap<String, String>) -> Result<Store, String> {
    let db = PathBuf::from(arg(m, "db")?);
    match access_of(cmd) {
        Some(Access::Rw) => Store::open(&db).map_err(|e| e.to_string()),
        Some(Access::Ro) => Store::open_read_only(&db).map_err(|e| e.to_string()),
        None => Err(format!("unknown command: {cmd}")),
    }
}

fn usage() {
    eprintln!("usage: neural-memory-admin <command> --db <path> [options]");
    eprintln!(
        "commands: {}",
        COMMANDS
            .iter()
            .map(|(c, _)| *c)
            .collect::<Vec<_>>()
            .join(" ")
    );
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.is_empty() {
        usage();
        return ExitCode::from(2);
    }
    let cmd = argv[0].clone();
    // Gate BEFORE run(). An arm that is not in COMMANDS is unreachable, so the
    // catch-all below it is dead in production and is only what the test asserts
    // against. Exit 2 like every other usage error, distinct from a command that
    // ran and failed.
    if access_of(&cmd).is_none() {
        eprintln!("unknown command: {cmd}");
        usage();
        return ExitCode::from(2);
    }
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
    // The store is opened INSIDE each arm, not here. Two reasons, both
    // load-bearing: a read-only command cannot be read-only if the open is
    // shared with the writers, and an unknown command must reach the catch-all
    // rather than failing first on `--db is required` -- which would make the
    // dispatch test below assert nothing.
    match cmd {
        "record-artifact" => {
            let store = open_for(cmd, m)?;
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
            let store = open_for(cmd, m)?;
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
            let store = open_for(cmd, m)?;
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
            let store = open_for(cmd, m)?;
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

        "retract" => {
            let store = open_for(cmd, m)?;
            let digest = arg(m, "digest")?;
            if !valid_sha256(&digest) {
                return Err("--digest must be 64 lowercase hex characters".into());
            }
            // Store::retract's UPDATE affects zero rows for an unknown digest and
            // returns Ok, so without this the CLI reports success for a typo.
            if store
                .get_record(&digest)
                .map_err(|e| e.to_string())?
                .is_none()
            {
                return Err(format!("no such record: {digest}"));
            }
            let reason = arg(m, "reason")?;
            store
                .retract(&digest, &reason, &arg(m, "at")?)
                .map_err(|e| e.to_string())?;
            Ok(format!("{digest} retracted: {reason}"))
        }

        "declare-standing" => {
            let store = open_for(cmd, m)?;
            let digest = arg(m, "digest")?;
            if !valid_sha256(&digest) {
                return Err("--digest must be 64 lowercase hex characters".into());
            }
            // --at is required, and not as a matter of taste: it lands in
            // standing_from, and the pair trigger aborts when standing_from and
            // standing_seq disagree about being NULL. arg() turns a raw SQL abort
            // about "half an interval" into a usable message.
            //
            // Its meaning differs from record-decision's --at: this is when the
            // rule BEGAN TO BIND, not when the record was written.
            store
                .declare_standing(WriteChannel::Operator, &digest, &arg(m, "at")?)
                .map_err(|e| e.to_string())?;
            // Report the headroom the operator just spent, so the next
            // declaration's refusal is not the first news of the budget.
            let held = store.standing_in_force().map_err(|e| e.to_string())?;
            let used: usize = held.iter().map(|(_, c)| c.chars().count()).sum();
            Ok(format!(
                "standing declared digest={digest} from={} ({} rule{}, {}/{} chars)",
                arg(m, "at")?,
                held.len(),
                if held.len() == 1 { "" } else { "s" },
                used,
                STANDING_BUDGET_CHARS
            ))
        }

        "standing" => {
            let store = open_for(cmd, m)?;
            let mut out = String::new();
            match m.get("as-of") {
                None => {
                    let held = store.standing_in_force().map_err(|e| e.to_string())?;
                    let used: usize = held.iter().map(|(_, c)| c.chars().count()).sum();
                    if held.is_empty() {
                        // The store was read successfully. That is a different
                        // fact from "the store could not be read", which
                        // open_read_only has already turned into an error above.
                        return Ok(format!(
                            "no standing rules in force (0/{STANDING_BUDGET_CHARS} chars)"
                        ));
                    }
                    out.push_str(&format!(
                        "{} rule{} in force, {}/{} chars\n",
                        held.len(),
                        if held.len() == 1 { "" } else { "s" },
                        used,
                        STANDING_BUDGET_CHARS
                    ));
                    for (digest, claim) in &held {
                        // Full digest on its own line: the workflow is read,
                        // pick one, paste into retract --digest. A truncation
                        // forces a second lookup at the one moment someone is
                        // choosing what to cut.
                        out.push_str(&format!("\n  {digest}\n    {claim}\n"));
                    }
                }
                Some(instant) => {
                    // A non-resolved instant is reported, never rendered as an
                    // empty set: "nobody can say" and "nothing was in force" are
                    // different answers and must not print alike.
                    let seq = match store.seq_at(instant).map_err(|e| e.to_string())? {
                        SeqAt::Resolved { seq } => seq,
                        SeqAt::BeforeAnyRecord => {
                            return Ok(format!(
                                "cannot answer: the store held nothing at or before {instant}"
                            ))
                        }
                        SeqAt::UnknownBefore { earliest_known_seq } => {
                            return Ok(format!(
                                "cannot answer: records exist at or before {instant} but carry no \
                                 recorded_at (earliest untimed seq {earliest_known_seq:?}); their \
                                 transaction time is an ordering only"
                            ))
                        }
                    };
                    let held = store.standing_as_of(seq).map_err(|e| e.to_string())?;
                    out.push_str(&format!(
                        "{} rule{} in force as of {instant} (sequence {seq})\n",
                        held.len(),
                        if held.len() == 1 { "" } else { "s" }
                    ));
                    for e in &held {
                        out.push_str(&format!(
                            "\n  {}\n    {}\n",
                            e.record_digest, e.claim
                        ));
                    }
                }
            }
            Ok(out.trim_end().to_string())
        }

        "stats" => {
            let store = open_for(cmd, m)?;
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
            let store = open_for(cmd, m)?;
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
            let store = open_for(cmd, m)?;
            let of = PathBuf::from(arg(m, "of")?);
            let diffs = verify_replica(&store, &of).map_err(|e| e.to_string())?;
            if diffs.is_empty() {
                Ok(format!("{} matches {} exactly", of.display(), arg(m, "db")?))
            } else {
                Err(format!(
                    "{} differs from {} in {} way(s):\n  {}",
                    of.display(),
                    arg(m, "db")?,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The gate's mirror hazard: a typo'd COMMANDS entry, or an arm removed
    /// while its name stays listed. Gating dispatch made the reverse
    /// (`dispatch ⊄ usage`) unreachable; this covers the direction that is left.
    #[test]
    fn every_advertised_command_dispatches() {
        for (c, _) in COMMANDS {
            let e = run(c, &HashMap::new()).unwrap_err();
            assert_ne!(
                e,
                format!("unknown command: {c}"),
                "{c} is advertised but not dispatched"
            );
        }
    }

    /// The banner is the only help text there is -- no --help, by choice -- so a
    /// command missing from it is undiscoverable. Mirrors the
    /// the_rust_budget_matches_the_trigger precedent of asserting against the
    /// source that must agree.
    ///
    /// First-token match, not `contains`: "standing" is a substring of
    /// "declare-standing", so a naive search would report the banner as
    /// documenting a command it never mentions.
    #[test]
    fn every_command_is_documented_in_the_banner() {
        let src = include_str!("admin.rs");
        let banner = src.split("\nuse ").next().unwrap();
        for (c, _) in COMMANDS {
            assert!(
                banner.lines().any(|l| {
                    l.trim_start_matches("//!").split_whitespace().next() == Some(*c)
                }),
                "{c} is missing from the //! Commands: banner"
            );
        }
    }

    /// The one that actually tests SQLITE_OPEN_CREATE.
    ///
    /// Moving the store open into each arm turned one wrong-by-default into N
    /// per-arm choices; the access column moved the choice back into data. This
    /// asserts the data is honoured, and the second assertion is the load-bearing
    /// one: a read-only command failing on a nonexistent path proves nothing on
    /// its own, since it would also fail for a dozen unrelated reasons. Only
    /// "the path still does not exist afterwards" distinguishes open_read_only
    /// from open, which would have created and migrated it and then answered
    /// over the empty store it just made.
    #[test]
    fn read_only_commands_do_not_create_the_database() {
        assert!(
            COMMANDS.iter().any(|(_, a)| *a == Access::Ro),
            "no command is classified Ro, so this test asserts nothing"
        );
        for (cmd, access) in COMMANDS {
            if *access != Access::Ro {
                continue;
            }
            let path = std::env::temp_dir().join(format!("nm-admin-ro-{cmd}.db"));
            let _ = std::fs::remove_file(&path);
            assert!(!path.exists(), "fixture path was not clean");

            let mut m = HashMap::new();
            m.insert("db".to_string(), path.display().to_string());
            let r = run(cmd, &m);

            assert!(r.is_err(), "{cmd} must refuse a nonexistent --db");
            assert!(
                !path.exists(),
                "{cmd} CREATED {} -- it opened read-write and would answer over an empty store",
                path.display()
            );
        }
    }

    /// Polarity for the above: a read-write command *does* create, so the test
    /// above is measuring the classification rather than something that happens
    /// to be true of every command.
    #[test]
    fn a_read_write_command_does_create_the_database() {
        let path = std::env::temp_dir().join("nm-admin-rw-probe.db");
        let _ = std::fs::remove_file(&path);
        let mut m = HashMap::new();
        m.insert("db".to_string(), path.display().to_string());
        m.insert("claim".to_string(), "a probe".to_string());
        m.insert("at".to_string(), "2026-08-29T00:00:00Z".to_string());

        assert_eq!(access_of("record-decision"), Some(Access::Rw));
        run("record-decision", &m).unwrap();
        assert!(path.exists(), "a read-write command must create its store");
        let _ = std::fs::remove_file(&path);
    }
}
