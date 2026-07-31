# M2 acceptance — does the semantic branch actually help? (2026-07-31)

> **Superseded in part.** `m3-retrieval-replication.md` re-ran this ablation at
> n = 30 per class, and re-ran the *identical 20 queries below* against a larger
> store with full embedding coverage. **The entity result replicated; the
> semantic result did not** — the same 20 queries now give +0.038 against the
> +0.050 threshold. Nothing below is edited: it records what was measured on the
> data available then, which is what an acceptance document is for.

Graded against `corpus/retrieval-eval.json`: 20 queries, relevance labels,
metric and pass conditions all fixed **before** any ablation ran.

## Result

Four arms differing only in which branches run. Macro-averaged per class.

| class | lexical | +semantic | +entity | all |
|---|---|---|---|---|
| | recall@5 / MRR | recall@5 / MRR | recall@5 / MRR | recall@5 / MRR |
| **A** verbatim | 1.00 / 1.00 | 1.00 / 1.00 | 1.00 / 1.00 | 1.00 / 1.00 |
| **B** paraphrase | 0.50 / 0.47 | **0.70 / 0.53** | 0.50 / 0.47 | 0.70 / 0.53 |
| **C** alias | 0.57 / 0.57 | 0.87 / 0.67 | **1.00 / 0.90** | 1.00 / 0.90 |
| **D** conceptual | 0.43 / 0.53 | **0.57 / 0.64** | 0.43 / 0.53 | 0.57 / 0.64 |

Against the pre-registered conditions:

- **Semantic: JUSTIFIED.** +0.087 MRR on paraphrase + conceptual (needed ≥ +0.050),
  and **exactly zero** cost on verbatim (allowed ≤ +0.050).
- **Entity: JUSTIFIED.** +0.333 MRR on alias (needed ≥ +0.050), zero cost elsewhere.

Marginal value of entity *given* semantic, which the pass conditions do not ask
about but which decides whether the branch is worth keeping:

| class | Δ MRR (all − lexical+semantic) |
|---|---|
| A verbatim | +0.000 |
| B paraphrase | +0.000 |
| **C alias** | **+0.233** |
| D conceptual | +0.000 |

Entity is not redundant. Semantic recovers some alias queries on its own
(0.57 → 0.67 MRR) but the declared dictionary is markedly better at them, and
the two do not overlap elsewhere.

**This is the first evidence in the project that either branch earns its weight.**
The earlier ad-hoc check — five queries, no labels — showed semantic surfacing
records lexical missed while changing the top hit on none of them, and I recorded
it as "not demonstrably useful". With labels and a metric the picture is
different, and better.

## The first instrument was contaminated

The first run went through the MCP surface, which hardcodes `entities: true`,
and approximated the ablation by discarding records whose branch list was
exactly `["entity"]`. That leaves records found by **both** semantic and entity
in place, still carrying their entity score. The "semantic only" arm was
therefore measuring semantic *plus part of* entity.

It mattered:

| | contaminated | clean |
|---|---|---|
| lexical MRR on C_alias | 0.70 | **0.57** |
| entity gain on C_alias | +0.200 | **+0.333** |
| `all` vs `lexical+semantic` | identical → "entity is redundant" | **+0.233 on C** |

The contaminated run inflated the lexical baseline and would have supported the
conclusion that the entity branch adds nothing once semantic is present. That
conclusion was wrong. The fix was a small binary driving `Store::recall` with
exact `RecallOptions` instead of post-filtering someone else's output.

## Limits, and they are severe

- **n = 5 per class.** One query moving from rank 1 to rank 2 changes that
  class's mean MRR by 0.10. The headline +0.087 on B+D is therefore *about one
  query's worth of movement*. The direction is consistent — semantic never hurts
  any class and helps two — but the magnitudes are not robust at this sample
  size and should not be quoted as effect sizes.
- **27 records.** recall@5 means retrieving 18% of the corpus, so the metric is
  a weak discriminator; almost any branch looks good at that ratio.
- **Same author wrote the corpus, the branches and the queries.** Paraphrases I
  chose may be ones this embedder happens to handle. Fixing the labels in
  advance and reporting per class limits the damage; it does not remove it.
- **No confidence intervals.** With n = 5 there is nothing honest to compute.

## What would make this convincing

A corpus and query set written by someone who did not build the branches, at
n ≥ 30 per class, over a corpus large enough that recall@5 is selective. Until
then this establishes that the branches behave as designed on their intended
query classes, not that they will help on anyone else's data.

## Non-claims

- No claim that these weights are optimal. They were derived from purpose, not
  tuned, and no tuning was attempted — tuning against a 20-query set this author
  wrote would fit the set, not the task.
- No claim about latency. The semantic branch adds an HTTP round trip per query
  and that was not measured here.
- No claim that entity extraction generalises. Recall is bounded by what was
  declared, and this dictionary was written for this corpus.
