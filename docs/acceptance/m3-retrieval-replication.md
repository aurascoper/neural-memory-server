# M3 acceptance — the ablation at n = 30, and what did not replicate (2026-07-31)

Graded against `corpus/retrieval-eval-v2.json`: 120 queries, 30 per class,
labels and pass conditions fixed **before** any v2 arm ran. Pass conditions are
carried over from `corpus/retrieval-eval.json` unchanged, so the two runs are
judged by the same bar.

The M2 result is in `m2-retrieval-ablation.md` and is **not** edited by this
document. It records what was measured then and stands as recorded.

## Headline: one branch replicated, the other did not

| | M2 (n=5) | M3 (n=30) | M3, the identical 20 v1 queries |
|---|---|---|---|
| semantic, MRR gain on B+D | **+0.087** ✅ | **+0.058** ✅ | **+0.038** ❌ |
| entity, MRR gain on C | **+0.333** ✅ | **+0.190** ✅ | **+0.233** ✅ |
| entity given semantic, C | +0.233 | +0.178 | +0.193 |

Threshold for both is ≥ +0.050.

- **Entity: replicated.** Weaker than at n = 5, comfortably clear of the bar on
  every cut, and the marginal value over semantic alone holds.
- **Semantic: did not replicate.** Run the *identical 20 queries* from M2 against
  the store as it is now and the gain is +0.038 — below the pre-registered
  threshold. The M2 conclusion was correct about the data it had and does not
  survive contact with more of it.

At n = 30 semantic clears the bar by 0.008. That is a hair, not a result. It
should be read as "the direction is still positive and the magnitude is not
established", not as a confirmation.

## Full table, n = 30 per class

| class | lexical | +semantic | +entity | all |
|---|---|---|---|---|
| | recall@5 / MRR | recall@5 / MRR | recall@5 / MRR | recall@5 / MRR |
| **A** verbatim | 1.000 / 1.000 | 1.000 / 1.000 | 1.000 / 1.000 | 1.000 / 1.000 |
| **B** paraphrase | 0.600 / 0.494 | **0.700 / 0.551** | 0.600 / 0.494 | 0.650 / 0.546 |
| **C** alias | 0.639 / 0.534 | 0.700 / 0.581 | **0.794 / 0.724** | 0.778 / 0.758 |
| **D** conceptual | 0.354 / 0.470 | 0.354 / **0.528** | 0.354 / 0.470 | 0.354 / 0.494 |

Verbatim is still 1.000 across every arm. That is not a good sign for the
metric — it means class A discriminates nothing and has not since M2.

## Why the replication subset exists

Three things changed between the runs, and without the subset a difference could
be attributed to any of them:

1. **n went 5 → 30 per class.**
2. **The store grew 75 → 92 records** (the M3 durability and concurrency
   evidence, written by this same milestone).
3. **Embedding coverage went 26 records → all 92.**

Re-running the identical 20 queries holds (1) fixed. It still moved, and moved
*down*, so the semantic weakening is **not** a sample-size effect — it is (2),
(3), or both.

**This run cannot separate (2) from (3), because they changed together.** Both
plausibly hurt: 17 new records are 17 new distractors, and going from partial to
full embedding coverage means the semantic branch is no longer searching a
subset of the corpus that happened to exclude most competitors. The second is
the more troubling reading, because under partial coverage the semantic arm was
being scored on an easier problem than the lexical arm without that being
visible anywhere in the M2 write-up. It is stated here as a candidate cause, not
a demonstrated one; separating them needs a run that changes coverage alone.

## A cost the pass conditions do not test

The pre-registered entity condition compares `lexical+entity` against `lexical`.
It never looks at what entity does *once semantic is already on*:

| class | Δ MRR (all − lexical+semantic) | at M2 |
|---|---|---|
| A verbatim | +0.000 | +0.000 |
| B paraphrase | **−0.005** | +0.000 |
| **C alias** | **+0.178** | +0.233 |
| D conceptual | **−0.033** | +0.000 |

At n = 5 these were exactly zero outside C. At n = 30 entity slightly *hurts*
conceptual and paraphrase queries when semantic is present. The magnitudes are
small and the pass conditions are not retroactively changed to catch this — that
would be editing the rubric after seeing the result. It is recorded as a finding
for the next pre-registration, not scored here.

## What the corpus expansion did and did not fix

Fixed:

- **One query no longer moves a class mean by 0.10.** It now moves it by 0.033,
  so the numbers are quotable as directions where before they were not.
- **Labels are record digests, not positions.** v1 labelled by index into
  `ORDER BY recorded_seq`. Appends never shifted those indices so v1 was not
  wrong, but it was one supersede away from silently mislabelling.
- **Duplicate claim texts are handled.** Ten claim texts exist at two digests
  each — same wording, different observation wiring, therefore different seals.
  A label on one is expanded to both by the builder, so no arm is penalised for
  returning a correct answer.
- **Retired records cannot be labelled.** Default recall excludes them, so a
  retired target would be unreachable and would depress every arm equally —
  uniform failure that reads as a finding and is an instrument defect.

Not fixed, and the M2 wording stands verbatim:

> **Same author wrote the corpus, the branches and the queries.** Paraphrases I
> chose may be ones this embedder happens to handle. Fixing the labels in
> advance and reporting per class limits the damage; it does not remove it.

Expanding from 20 queries to 120 written in the same sitting by the same author
samples one person's idea of a paraphrase six times instead of once. **n was the
smaller of the two limits and it is the one that got fixed.**

There is now a sharper version of this for the alias class specifically, recorded
in `corpus/entities-m3.toml` and in the eval file: an alias query is a query the
author wrote using an alias the author also wrote. The entity dictionary was
extended from 6 entities to 24 *before* any v2 query was authored, which prevents
the worst form of tuning — choosing entities after seeing which queries fail —
but it does not make C_alias a measurement of anything but "does the branch work
as designed".

## Still true from M2, still unfixed

- **recall@5 is a weak discriminator.** 5 of 92 is 5.4%, better than the 18% M2
  reported against, but class A remains at 1.000 in every arm.
- **No confidence intervals.** n = 30 would support them; they were not
  pre-registered, and computing them now and quoting them would be choosing a
  statistic after seeing the data.
- **No claim these weights are optimal.** Derived from purpose, never tuned. The
  temptation to nudge `W_SEMANTIC` up now that semantic looks marginal is exactly
  the tuning-against-the-test-set this discipline exists to prevent.

## What would still make this convincing

Unchanged from M2, and now with one item struck through:

- ~~n ≥ 30 per class~~ — done.
- A corpus and query set written by **someone who did not build the branches**.
- A corpus large enough that recall@5 is selective, which 92 records is not.
- A run that varies embedding coverage alone, to settle whether the M2 semantic
  result was an artefact of the semantic branch searching a third of the corpus.
