# The queries that justify two time axes

`Correction 5` in the plan deferred bitemporality until *"a specific query demands
it, and that query is written down first"*. This is that document. It is written
before the migration, so the design answers a stated need rather than the need
being invented to fit the design.

## The distinction

- **Valid time** — when a claim was true of the world. `occurred_at`. A benchmark
  run at 14:03 has that as its valid time regardless of when it was recorded.
- **Transaction time** — when the store came to believe it. `recorded_seq`, which
  is monotonic and therefore *is* a transaction-time axis with no clock involved.

An append-only log answers "what do we believe now" without either. These three
queries need both.

## Q1 — Was a past decision reasonable on the evidence then available?

> The support-matrix row was written on 2026-07-30. What did the store say about
> the Vulkan contract blocker **at that point**?

Today the answer is "blocked by the measurement corpus". On 2026-07-30 it was
"blocked because `BackendConformancePolicy` has no backend dimension" — a claim
since retired. Auditing the decision requires the belief as it stood, not the
belief as corrected. Without transaction time you cannot distinguish *"they were
careless"* from *"they were right on what was known"*, and that distinction is
the whole point of keeping the retired claim rather than deleting it.

## Q2 — Which results were reported under an instrument later found broken?

> Three quality-harness checkers were found defective. Which scores had already
> been reported when each defect was still unknown?

Valid time is the measurement window; transaction time is when the defect was
discovered. The set of *"reported before we knew"* is exactly the intersection,
and it is the set that has to be re-examined. Neither axis alone identifies it:
`occurred_at` alone cannot say what was known, and `recorded_seq` alone cannot
say what the score was about.

## Q3 — Did this claim rest on evidence that was current when it was made?

> Claim C cites observation O. Was O already superseded when C was written?

A claim resting on evidence retired *before* the claim existed is a defect. A
claim resting on evidence retired *afterwards* is ordinary history and needs
only a note. Distinguishing them is a comparison of two transaction times, and
without one the store cannot tell a stale citation from a superseded one.

## What is deliberately NOT built

No `tstzrange`, no exclusion constraints, no Allen-relation predicates. SQLite
has no native bitemporal support and hand-rolling six-predicate temporal joins
would be a large amount of machinery for three questions. What these need is a
sequence comparison, and that is what they get.

## The honest gap

Records written before migration 0003 carry **no `recorded_at` timestamp** —
their transaction time is known as an *ordering* (`recorded_seq`) but not as a
wall-clock instant. `seq_at()` therefore distinguishes "no records that early"
from "transaction time unknown for records that early", rather than inventing a
timestamp for them. Backfilling `occurred_at` into `recorded_at` would conflate
the two axes this document exists to separate.
