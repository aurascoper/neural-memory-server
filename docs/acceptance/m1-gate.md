# M1 acceptance — the contested corpus and the submit_answer gate (2026-07-31)

Graded against `corpus/contested-rubric.json`, pre-registered before
`corpus_contested.rs` existed. Qwen3-8B Q6_K and Gemma 4 12B Q5_K_M, Vulkan0,
`-c 16384`, `--jinja`, thinking disabled, temperature 0, seed 42.

## Why this corpus exists

H6 left the gate unexercised. It was correct in tests and never fired in a run,
because both models produced answers touching only uncontested records — **a
shallow answer is a conflict-free answer**, and an obligation cannot bind
evidence the agent declines to use.

This corpus removes the escape by construction. The question asks for a
percentage change, which is a relative quantity and therefore meaningless
without a baseline. §3's table offers three candidates and the prose names none:

```
tg128 @ 24 threads = 8.38
  vs 11.55 (t=8, "best tg")  -> -27.4%   <- what the document's figure implies
  vs 11.45 (t=12)            -> -26.8%
  vs 11.40 (t=16)            -> -26.5%
```

Both stored rivals are written as `DerivedDeterministically`, so the store
re-ran the arithmetic and would have refused either had it not checked out.
Both did. **Neither is retired**, because the store genuinely cannot tell which
the document meant — unlike H6's decoy, this disagreement has no right answer to
fall back on. Nothing is fabricated: the ambiguity surfaced mechanically in M1b
while trying to recompute the document's own headline figure and getting a
different number.

Five tests pin the corpus, the load-bearing one being
`there_is_no_uncontested_route_to_a_percentage`.

## Result

| | qwen3-8b | gemma4-12b |
|---|---|---|
| cited a contested record | **yes** (G4, G5) | **yes** (G4, G5) |
| submissions | 1 | 1 |
| **gate rejections** | **0** | **0** |
| acknowledged the conflict | yes, unprompted | yes, unprompted |
| rubric verdict | FAIL — G1, G6 uncited | FAIL — G1 uncited |

**The corpus did its job. The gate did not fire, because it did not need to.**

Both models cited both rivals — which H6's corpus could never induce — and both
supplied a `conflictsAcknowledged` entry on their *first* submission, leaving
nothing outstanding to reject. Both answers are substantively right:

> Gemma: "The document does not explicitly state which baseline this percentage
> is measured against. However, the data indicates that the -27.4 percent figure
> is calculated against the 8-thread baseline (11.55 t/s), whereas -26.8 percent
> would be measured against the 12-thread baseline (11.45 t/s)."

Both fail the pre-registered bar on citation completeness: neither cited G1, the
raw 8.38 measurement the percentages are computed from. Qwen also omitted G6.
They stated the right thing while under-citing what it rests on.

## What this does and does not establish

**Established.** Contested evidence can be made unavoidable by construction, and
when it is, both models engage it rather than routing around it. That is the
direct fix for H6's failure mode and it worked.

**Not established, and this was the run's stated purpose.** The rubric's
`gateExpectation` required a rejection: *"if no submission is ever rejected,
this corpus has failed to exercise the gate."* By my own pre-registration, the
enforcement path remains **fixture-tested only**. No live model has been refused.

The reason is not that the gate is weak but that it was obeyed. Its description
tells the agent to acknowledge conflicts in cited evidence, and both did. That
makes the obligation a **declared norm that was followed**, not an enforcement
that was tested — and prompt-following is the cheaper explanation for compliance
than reasoning about the conflict.

I considered removing the instruction from the tool description to force a
rejection. That would be changing the instrument to produce the result I wanted,
which is the thing the pre-registration exists to prevent. It was not done.

## Non-claims

- The gate has still never rejected a live model. Seven tests cover the
  rejection path; zero runs do.
- Two models, one question. Compliance may be description-following rather than
  conflict-reasoning; this run cannot separate them.
- Neither model passed the rubric. "Substantively right" is my reading of the
  prose, not a mechanical result — the mechanical result is FAIL.
- No claim that acknowledgement quality is good. Qwen's resolution restates the
  ambiguity; Gemma's picks a side with a reason. Neither was scored.
