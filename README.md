# neural-memory-server

A provenance-preserving evidence store for benchmark results and the claims
built on them. Records are addressable, citations are mechanically checkable,
and a retired claim does not come back with equal standing to its replacement.

**Status: M1, a walking skeleton. Ready to use, not ready to depend on.** See
[Known limits](#known-limits).

---

## Why this exists

It was originally justified on latency — retrieve small targeted context instead
of stuffing whole documents into prompts. **That justification was measured and
falsified before any of this was written.** On the target hardware
(Radeon 890M, RADV/Vulkan, llama.cpp `d0bfb1981`, r=5):

| workload | prefill | TTFT |
|---|---:|---:|
| 8K, distinct content each turn | 8000 | 41.57 s |
| 8K, stable 7K prefix + 1K tail | **1000** | **5.83 s** |
| 2K, distinct content each turn | 2000 | 8.78 s |

A stable-prefix 8K prompt reaches first token **33% faster than a cold 2K one**.
Prompt-prefix caching is on by default; per-query retrieval is *cache-hostile*.
So reducing context is the wrong objective, and the project was re-argued on
what survives — the things prose plus `grep` genuinely cannot do:

- **Addressable units.** "Cite record X" is checkable; a citation into prose is not.
- **Retirement that retrieval respects.** Superseded claims are withheld by
  default, reported as withheld, and still retrievable on request.
- **Evidence class derived from structure**, never asserted by the writer.
- **A referent constraint that is enforced.** A relative quantity with no named
  reference is *unstorable*, not merely discouraged.

No latency or context-size claim is made for this software.

## Quickstart

```sh
cargo build --release

# Author evidence as a document, check it, then apply it.
./target/release/neural-memory-ingest --db store.db --file corpus/contested.toml --dry-run
./target/release/neural-memory-ingest --db store.db --file corpus/contested.toml

./target/release/neural-memory-admin stats --db store.db
```

Run as an MCP server over stdio. `--as-of` is required and has no default: the
store never reads a clock, so recency is reproducible and the reference instant
is a recorded session parameter rather than an ambient one. The shell supplies
it.

```sh
./target/release/neural-memory-mcp --db store.db --as-of "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
```

## Adding evidence

Evidence is a TOML document (see `corpus/contested.toml`). TOML rather than JSON
because evidence needs comments — where a figure came from is often more
important than the figure.

```toml
version = 1
recorded_at = "2026-07-31T00:00:00Z"

[[observation]]
id = "t24"                  # local alias; the importer resolves it to a digest
kind = "tg128.threads24"
quantity = "absolute"       # "relative" REQUIRES a reference
value = "8.38"              # quoted: a bare number is a float, and a float in a
policy = "tgps"             #   sealed document is at the serialiser's mercy
suite = "sweep"
runtime = "llama.cpp-b10188-d0bfb1981"

[[claim]]
id = "g4"
text = "Generation at 24 threads changes by -27.4 percent against the 8-thread baseline"
evidence = "derived"        # the store RE-RUNS the arithmetic and refuses a mismatch
observations = ["t24", "t8"]
[claim.derivation]
transform = "percentChange"
value = "t24"
baseline = "t8"
decimals = 1
```

The format cannot be used to bypass the rules. `evidence = "observed"` requires
an artifact that exists; `"derived"` requires a transform that recomputes;
unknown fields are rejected rather than ignored, because a typo'd key silently
dropped is evidence quietly not recorded.

## Architecture

```
neural-memory-domain   PURE. no database, no async runtime, no clock, no uuid.
                       Sealed identities, sort discipline, the assembler.
neural-memory-store    SQLite + FTS5. migrations, retrieval, ingestion.
neural-memory-mcp      typed MCP tools + operator CLIs.
```

Dependencies run one way: `mcp → store → domain`. Purity is enforced by
`scripts/check-purity.sh` against the resolved dependency tree, not by intent —
the UUID ban matters most, because a primary key mistaken for an identity fails
silently. (The schema goes further and contains no UUIDs at all: a record's key
*is* its content digest.)

### The agent surface is split by reachability

| reachable by a model | operator only |
|---|---|
| `recall`, `get_record`, `trace_provenance` | `record-artifact`, `record-observation` |
| `remember` — always `agentInference` | `record-decision`, `supersede`, `ingest` |
| `flag_contradiction`, `submit_answer` | |

`supersede` is deliberately out of reach: an agent that can be prompt-injected
into retiring true records fails worse than one that cannot retire anything. The
agent gets `flag_contradiction`, which records a reviewable edge and retires
nothing. There is no SQL tool.

`submit_answer` is **checked, not trusted**: if a cited record has an unresolved
conflict, submission is rejected until it is acknowledged.

### The assembler is append-only

Following the measurement above, context assembly never re-emits, reorders or
re-summarises. A record already in the session prefix is not sent again; a
supersession *appends* the replacement and leaves the retired record in place,
because rewriting history would invalidate the prefix and erase the evidence
that the belief changed. The metric is `total_prefill_tokens` over a session,
never `peak_context_tokens` — a design that optimises the latter while inflating
the former looks successful and is not.

## Testing

128 tests. `cargo test`, `cargo fmt --check`, `cargo clippy --all-targets -D
warnings`, `./scripts/check-purity.sh`.

Two conventions carried from `neuralcompose-client-native`:

- **Both polarities.** A gate that rejects everything passes a rejection test; a
  digest that ignores a field passes an invariance test. Only the pair says
  anything.
- **Uniform failure means a broken instrument.** A result identical across all
  candidates is far more likely a defective checker than unanimous agreement.
  This caught three checker bugs and one void experiment run.

## Findings

`docs/acceptance/` records what the store was used to establish, including where
it failed:

- **[m1-h6.md](docs/acceptance/m1-h6.md)** — can a fresh agent reconstruct why a
  numerical contract is blocked? Nothing passed the pre-registered bar. Making a
  conflict *reachable*, then *visible*, then *binding* did not change the
  outcome, because a shallow answer is also a conflict-free answer.
- **[m1-gate.md](docs/acceptance/m1-gate.md)** — a corpus whose answer cannot
  avoid contested evidence. It fixed H6's failure mode; the gate still has not
  rejected a live model, because both models complied pre-emptively.

Rubrics in `corpus/*.json` are pre-registered before the corresponding importer
exists, and are void if edited after a grading run.

## Known limits

- **M2 is unbuilt by design**: no embeddings or vector search, no entity
  retrieval branch, no temporal queries, no as-of time travel.
- **Concurrency is untested.** One SQLite file, WAL mode, multiple clients.
- **No backup procedure.** Backup is copying the file; that is not the same as
  having tested a restore.
- **The corpus is small** — tens of claims, drawn from one characterization
  document. Absence from the store is not evidence of absence in the world.
- **`submit_answer`'s rejection path has never fired against a live model.**
  Seven tests cover it; zero runs do.

## Licence

MIT.
