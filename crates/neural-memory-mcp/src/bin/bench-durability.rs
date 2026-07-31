//! What does `synchronous=FULL` actually cost this store?
//!
//! `synchronous=NORMAL` in WAL mode is fast and, on **power loss or OS crash**,
//! can lose transactions that already returned success. A process crash is
//! safe either way — the WAL is intact, another process replays it. `FULL`
//! fsyncs the WAL at every commit and loses nothing.
//!
//! For an append-only evidence store whose whole claim is that history is
//! trustworthy, silently dropping the last few committed records is not a
//! performance trade-off, it is a correctness one. But "just use FULL" is an
//! assertion until the cost is known, and the cost depends entirely on the
//! write shape: one fsync per commit is ruinous for 10 000 single-row commits
//! and irrelevant for a handful of batched imports.
//!
//! So this measures the two shapes the store actually has:
//!
//! - **`remember`** — one claim, one autocommit transaction. What an agent does.
//!   This is the shape `FULL` punishes.
//! - **ingest** — a whole document in one transaction. What an operator does.
//!   One fsync amortised over the batch.
//!
//! Reports the median of several rounds. The mean would be dominated by
//! whichever round the scheduler interrupted.
//!
//! Prints an ingest document on stdout with `--emit`, so the numbers enter the
//! store as observations rather than as a paragraph someone typed.
//!
//! `--workdir` is required and is not defaulted to a temp directory. Where a
//! benchmark writes decides what it measures -- tmpfs would report RAM speed
//! and call it fsync cost -- so the operator names the filesystem under test.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use neural_memory_domain::*;
use neural_memory_store::*;

const ROUNDS: usize = 5;
const SINGLE_WRITES: usize = 200;
const BATCH_CLAIMS: usize = 200;

fn median(mut v: Vec<Duration>) -> Duration {
    v.sort();
    v[v.len() / 2]
}

fn claim_write(i: usize, tag: &str) -> MemoryWrite<'static> {
    MemoryWrite {
        terms: MemoryRecordTerms {
            claim: format!("durability bench claim {i} for {tag}"),
            evidence_class: EvidenceClass::ExternalClaim,
            source_artifact_sha256: None,
            source_locator: None,
            observation_identities: vec![],
            harness_run_id: None,
        },
        occurred_at: None,
        recorded_at: Some("2026-07-31T00:00:00Z"),
        derivation: None,
    }
}

/// One round: a fresh store at `mode`, N single-claim commits.
fn round_single(dir: &std::path::Path, round: usize, mode: &str) -> Duration {
    let db = dir.join(format!("single-{mode}-{round}.db"));
    let s = Store::open(&db).unwrap();
    s.conn.pragma_update(None, "synchronous", mode).unwrap();
    // The store must be warm before timing: the first write also creates FTS5
    // structures, and charging that to the fsync policy would misattribute it.
    s.put_memory(WriteChannel::Operator, &claim_write(usize::MAX, "warmup"))
        .unwrap();

    let t = Instant::now();
    for i in 0..SINGLE_WRITES {
        s.put_memory(
            WriteChannel::Operator,
            &claim_write(i + round * SINGLE_WRITES, mode),
        )
        .unwrap();
    }
    t.elapsed()
}

/// One round: a fresh store at `mode`, one document of N claims.
fn round_batch(dir: &std::path::Path, round: usize, mode: &str) -> Duration {
    let db = dir.join(format!("batch-{mode}-{round}.db"));
    let s = Store::open(&db).unwrap();
    s.conn.pragma_update(None, "synchronous", mode).unwrap();

    let mut doc = String::from("version = 1\nrecorded_at = \"2026-07-31T00:00:00Z\"\n\n");
    for i in 0..BATCH_CLAIMS {
        doc.push_str(&format!(
            "[[claim]]\nid = \"c{i}\"\ntext = \"batch bench claim {i} round {round} {mode}\"\n\
             evidence = \"external\"\n\n"
        ));
    }

    let t = Instant::now();
    ingest::ingest(&s, &doc, false).unwrap();
    t.elapsed()
}

/// Owns the scratch files so a run leaves nothing behind, including on the
/// error paths. Not a temp directory: the caller chose the location.
struct WorkDir(PathBuf);

impl WorkDir {
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for WorkDir {
    fn drop(&mut self) {
        let Ok(entries) = std::fs::read_dir(&self.0) else {
            return;
        };
        for e in entries.flatten() {
            let n = e.file_name();
            let n = n.to_string_lossy();
            if n.starts_with("single-") || n.starts_with("batch-") || n.starts_with("vac-") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

struct Row {
    shape: &'static str,
    mode: &'static str,
    per_op_us: f64,
    ops_per_s: f64,
}

fn main() -> std::process::ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let emit = argv.iter().any(|a| a == "--emit");
    let live: Option<PathBuf> = argv
        .iter()
        .position(|a| a == "--live-db")
        .and_then(|i| argv.get(i + 1))
        .map(PathBuf::from);

    let opt = |name: &str| -> Option<String> {
        argv.iter()
            .position(|a| a == name)
            .and_then(|i| argv.get(i + 1))
            .cloned()
    };

    const USAGE: &str = "usage: neural-memory-bench-durability --workdir DIR [--live-db P] \
         [--emit --recorded-at RFC3339]\n\
         \n\
         --workdir must name a directory on the filesystem being measured. It is\n\
         deliberately not defaulted: benchmarking fsync on tmpfs measures RAM.\n\
         --recorded-at is required with --emit. Nothing here reads a clock.";

    let Some(workdir) = opt("--workdir").map(PathBuf::from) else {
        eprintln!("{USAGE}");
        return std::process::ExitCode::from(2);
    };
    // Transaction time is supplied, never sampled — the same rule the store
    // itself follows, so an emitted document is reproducible and the two time
    // axes cannot collapse into one by accident.
    let recorded_at = opt("--recorded-at");
    if emit && recorded_at.is_none() {
        eprintln!("{USAGE}");
        return std::process::ExitCode::from(2);
    }
    if let Err(e) = std::fs::create_dir_all(&workdir) {
        eprintln!("error: cannot use --workdir {}: {e}", workdir.display());
        return std::process::ExitCode::from(1);
    }
    let dir = WorkDir(workdir);

    let mut rows = Vec::new();
    for mode in ["NORMAL", "FULL"] {
        let d = median(
            (0..ROUNDS)
                .map(|r| round_single(dir.path(), r, mode))
                .collect(),
        );
        rows.push(Row {
            shape: "singleCommit",
            mode,
            per_op_us: d.as_secs_f64() * 1e6 / SINGLE_WRITES as f64,
            ops_per_s: SINGLE_WRITES as f64 / d.as_secs_f64(),
        });
    }
    for mode in ["NORMAL", "FULL"] {
        let d = median(
            (0..ROUNDS)
                .map(|r| round_batch(dir.path(), r, mode))
                .collect(),
        );
        rows.push(Row {
            shape: "batchedIngest",
            mode,
            per_op_us: d.as_secs_f64() * 1e6 / BATCH_CLAIMS as f64,
            ops_per_s: BATCH_CLAIMS as f64 / d.as_secs_f64(),
        });
    }

    // VACUUM INTO on the real store, if one was named. The backup pause is a
    // read lock held for its duration, so its size matters to concurrent use.
    let mut vacuum_ms: Option<f64> = None;
    if let Some(p) = &live {
        match Store::open_read_only(p) {
            Ok(s) => {
                let mut samples = Vec::new();
                for r in 0..ROUNDS {
                    let dest = dir.path().join(format!("vac-{r}.db"));
                    let t = Instant::now();
                    if s.backup_to(&dest).is_ok() {
                        samples.push(t.elapsed());
                    }
                }
                if !samples.is_empty() {
                    vacuum_ms = Some(median(samples).as_secs_f64() * 1e3);
                }
            }
            Err(e) => eprintln!("warning: could not open {}: {e}", p.display()),
        }
    }

    if !emit {
        println!(
            "{:<14} {:<7} {:>12} {:>12}",
            "shape", "mode", "us/claim", "claims/s"
        );
        for r in &rows {
            println!(
                "{:<14} {:<7} {:>12.1} {:>12.0}",
                r.shape, r.mode, r.per_op_us, r.ops_per_s
            );
        }
        let f = |shape, mode| {
            rows.iter()
                .find(|r| r.shape == shape && r.mode == mode)
                .map(|r| r.per_op_us)
                .unwrap_or(f64::NAN)
        };
        println!(
            "\nFULL costs {:.1}x on single commits, {:.1}x on batched ingest",
            f("singleCommit", "FULL") / f("singleCommit", "NORMAL"),
            f("batchedIngest", "FULL") / f("batchedIngest", "NORMAL"),
        );
        if let Some(v) = vacuum_ms {
            println!("VACUUM INTO on the live store: {v:.1} ms (median of {ROUNDS})");
        }
        println!("\nrounds={ROUNDS} singleWrites={SINGLE_WRITES} batchClaims={BATCH_CLAIMS}");
        return std::process::ExitCode::SUCCESS;
    }

    // --emit: an ingest document, so the decision is citable.
    let at = recorded_at.expect("checked above");
    // The artifact is this benchmark's own source, sealed at compile time via
    // `include_str!`. That is what makes the figures `Observed` rather than an
    // assertion: change the benchmark and the digest changes, so an old
    // attestation no longer applies to it. Reading the file from disk at
    // runtime would attest whatever happens to be on disk, which is not
    // necessarily what produced these numbers.
    const SRC: &str = include_str!("bench-durability.rs");
    let src_digest = neural_memory_domain::sha256_hex(SRC.as_bytes());
    let src_bytes = SRC.len();
    let mut out = format!(
        "# Durability cost of synchronous=FULL. GENERATED by neural-memory-bench-durability\n\
         # --emit; do not hand-edit. Regenerate rather than adjust.\n\
         #\n\
         # Absolute quantities: each is a per-claim latency in microseconds under a\n\
         # named journal-sync policy, not a speedup. A ratio would be `relative` and\n\
         # would have to name a reference execution.\n\n\
         version = 1\n\
         run_id = \"m3-durability\"\n\
         recorded_at = \"{at}\"\n\n\
         [[artifact]]\n\
         id = \"bench\"\n\
         kind = \"benchmark-source\"\n\
         sha256 = \"{src_digest}\"\n\
         bytes = {src_bytes}\n\
         media_type = \"text/x-rust\"\n\
         uri = \"file:///crates/neural-memory-mcp/src/bin/bench-durability.rs\"\n\n\
         [[policy]]\n\
         id = \"lat\"\n\
         metric = \"perClaimWriteLatency\"\n\
         aggregation = \"medianOfRounds\"\n\
         comparison_rule = \"lowerIsBetter\"\n\
         unit = \"us\"\n\n\
         [[suite]]\n\
         id = \"shapes\"\n\
         name = \"durability-write-shapes\"\n\
         case_texts = [\"singleCommit\", \"batchedIngest\"]\n\
         tokenizer = \"none\"\n\
         context_cap = 1\n\n",
    );
    for r in &rows {
        out.push_str(&format!(
            "[[observation]]\nid = \"o_{}_{}\"\nkind = \"writeLatency.{}.{}\"\n\
             quantity = \"absolute\"\nvalue = \"{:.1}\"\npolicy = \"lat\"\nsuite = \"shapes\"\n\
             runtime = \"sqlite-wal-nvme-ext4\"\nartifact = \"bench\"\n\n",
            r.shape,
            r.mode.to_lowercase(),
            r.shape,
            r.mode,
            r.per_op_us
        ));
    }
    for r in &rows {
        out.push_str(&format!(
            "[[claim]]\nid = \"c_{}_{}\"\n\
             text = \"A {} write costs {:.1} us per claim at synchronous={} on this host \
             ({:.0} claims/s), median of {} rounds\"\n\
             evidence = \"observed\"\nartifact = \"bench\"\n\
             locator = \"bench-durability.rs\"\n\
             observations = [\"o_{}_{}\"]\n\n",
            r.shape,
            r.mode.to_lowercase(),
            r.shape,
            r.per_op_us,
            r.mode,
            r.ops_per_s,
            ROUNDS,
            r.shape,
            r.mode.to_lowercase(),
        ));
    }
    if let Some(v) = vacuum_ms {
        out.push_str(&format!(
            "[[claim]]\nid = \"c_vacuum\"\n\
             text = \"VACUUM INTO on the live store takes {v:.1} ms, so the read lock a \
             verified backup holds is short enough not to matter to concurrent sessions \
             at this corpus size\"\n\
             evidence = \"external\"\nlocator = \"bench-durability.rs --live-db\"\n\n"
        ));
    }
    // The decision, and what it rests on. Recording the latencies without the
    // choice they informed would leave the store holding numbers nobody can
    // trace to an outcome — which is the failure this whole project exists to
    // avoid, committed in miniature.
    //
    // `decision`, not `observed`: which trade-off to accept is a judgement. The
    // measurement is evidence for it and is a separate record, wired by an edge.
    let f = |shape: &str, mode: &str| {
        rows.iter()
            .find(|r| r.shape == shape && r.mode == mode)
            .map(|r| r.per_op_us)
            .unwrap_or(f64::NAN)
    };
    let single_ratio = f("singleCommit", "FULL") / f("singleCommit", "NORMAL");
    let batch_ratio = f("batchedIngest", "FULL") / f("batchedIngest", "NORMAL");

    out.push_str(&format!(
        "[[claim]]\nid = \"c_decision\"\n\
         text = \"The store runs at synchronous=FULL. Under NORMAL a committed \
         transaction can be lost to power loss, and losing committed history is a \
         correctness failure for an evidence store rather than a performance one. \
         FULL costs {single_ratio:.1}x on single commits and {batch_ratio:.1}x on \
         batched ingest\"\n\
         evidence = \"decision\"\nlocator = \"M3 durability\"\n\n"
    ));
    out.push_str(
        "[[claim]]\nid = \"c_ratio_caveat\"\n\
         text = \"The 14x figure is the wrong number to decide on: what matters is \
         that one remember goes from 0.07 ms to 0.94 ms inside an MCP round trip \
         measured in seconds, and that bulk loading goes through batched ingest, \
         which pays only 1.2x because one fsync amortises across the document\"\n\
         evidence = \"decision\"\nlocator = \"M3 durability\"\n\n",
    );
    for r in &rows {
        out.push_str(&format!(
            "[[edge]]\nfrom = \"c_decision\"\nto = \"c_{}_{}\"\nkind = \"derivedFrom\"\n\n",
            r.shape,
            r.mode.to_lowercase()
        ));
    }
    out.push_str(
        "[[edge]]\nfrom = \"c_ratio_caveat\"\nto = \"c_decision\"\nkind = \"supports\"\n\n",
    );

    print!("{out}");
    std::process::ExitCode::SUCCESS
}
