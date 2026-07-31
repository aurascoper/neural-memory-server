//! Append-only, prefix-stable context assembly.
//!
//! Deterministic and effect-free: no I/O, no clock, no tokenizer. Token costs are
//! supplied by the caller.
//!
//! ## Why this is not a minimiser
//!
//! Measured on the target hardware (Qwen3-8B/Vulkan, r=5, prefill read from the
//! server's own `timings.prompt_n`):
//!
//! | workload                            | prefill | TTFT     |
//! |-------------------------------------|--------:|---------:|
//! | 8K, distinct content each turn      | 8000    | 41.57 s  |
//! | 8K, stable 7K prefix + 1K tail      | **1000**| **5.83 s** |
//! | 2K, distinct content each turn      | 2000    | 8.78 s   |
//!
//! A stable-prefix 8K prompt reaches first token **33% faster than a cold 2K
//! one**. Prefill runs at ~208 tok/s, so a context that varies every turn must
//! come in under ~1,200 fresh tokens to beat simply keeping a 7K prefix and
//! appending to it. Shrinking the context is therefore the wrong objective; a
//! design that halves `peak_context_tokens` while rebuilding every turn is
//! *slower* than one that never shrinks anything.
//!
//! So this assembler answers a different question. Not "what are the best N
//! tokens?" but **"given what the prefix already contains, what must be
//! appended?"**
//!
//! ## The rules
//!
//! - A record already in the prefix is **never re-emitted**.
//! - Emitted content is **never reordered, re-ranked, or re-summarised**. Any
//!   edit at token *k* invalidates the cache from *k* onward, so a "better
//!   ordering" discovered at turn 5 costs more than it saves.
//! - Within a turn, append order is **deterministic** (by digest), so the same
//!   append set always produces the same bytes.
//! - Supersession **appends** the replacement and leaves the retired record in
//!   place. Rewriting history to remove it would invalidate the prefix and
//!   destroy the evidence that the correction happened.
//! - The prefix breaks in exactly one circumstance — eviction — which is a
//!   countable cliff rather than a per-turn tax.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::digest::seal;

const SESSION_PREFIX_DOMAIN: &str = "neuralmemory.session-prefix.v1";

/// What the model has already been shown this session, in the order shown.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionPrefix {
    /// Record digests in append order. **ORDER SIGNIFICANT** — this is the byte
    /// order the model saw, and reordering it is exactly the cache invalidation
    /// this type exists to prevent.
    pub emitted: Vec<String>,
}

impl SessionPrefix {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn contains(&self, digest: &str) -> bool {
        self.emitted.iter().any(|e| e == digest)
    }

    pub fn len(&self) -> usize {
        self.emitted.len()
    }

    pub fn is_empty(&self) -> bool {
        self.emitted.is_empty()
    }

    /// Apply a plan. Appends only; existing entries are never touched.
    pub fn apply(&mut self, plan: &AppendPlan) {
        self.emitted.extend(plan.append.iter().cloned());
    }
}

/// Identity of a prefix state. Two sessions with the same identity have shown the
/// model the same bytes in the same order, so a cache built for one is valid for
/// the other. A changed identity means the prefix was invalidated.
pub fn session_prefix_identity(p: &SessionPrefix) -> String {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Doc<'a> {
        domain: &'static str,
        // NOT sorted: order is the meaning here.
        emitted: &'a [String],
    }
    seal(&Doc {
        domain: SESSION_PREFIX_DOMAIN,
        emitted: &p.emitted,
    })
}

/// What to append this turn, and what was deliberately withheld.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppendPlan {
    /// **ORDER SIGNIFICANT** — the order these will be written in. Deterministic
    /// by digest so the same set always yields the same bytes.
    pub append: Vec<String>,
    /// Retrieved but already present, so not re-emitted. **SORTED** — a set.
    /// Reported rather than dropped silently, so that "retrieval returned it and
    /// the model already has it" is distinguishable from "retrieval missed it".
    pub suppressed: Vec<String>,
    /// True when the budget forced the prefix to be discarded this turn.
    pub invalidated_prefix: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct AssemblerConfig {
    /// Total context the prefix may occupy before eviction forces a rebuild.
    pub context_budget_tokens: u32,
}

/// Decide what to append, given what the prefix already holds.
///
/// `retrieved` may be in any order and may contain duplicates; the result does
/// not depend on either.
pub fn plan_append(prefix: &SessionPrefix, retrieved: &[String]) -> AppendPlan {
    // BTreeSet does the dedupe and the deterministic ordering in one step.
    let unique: BTreeSet<&String> = retrieved.iter().collect();
    let mut append = Vec::new();
    let mut suppressed = Vec::new();
    for d in unique {
        if prefix.contains(d) {
            suppressed.push(d.clone());
        } else {
            append.push(d.clone());
        }
    }
    AppendPlan {
        append,
        suppressed,
        invalidated_prefix: false,
    }
}

/// Append a replacement for a retired record.
///
/// The retired record stays in the prefix. It is not removed, not rewritten, and
/// not reordered — removing it would invalidate every token after it *and* erase
/// the fact that the belief changed. The reader sees the original, then the
/// correction, in the order they were learned.
pub fn plan_supersession(prefix: &SessionPrefix, replacement: &str) -> AppendPlan {
    plan_append(prefix, std::slice::from_ref(&replacement.to_string()))
}

// ---------------------------------------------------------------------------
// Cost accounting — the evidence for H1'
// ---------------------------------------------------------------------------

/// Prefill cost of a whole session. The headline number is
/// `total_prefill_tokens`, never a peak.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionCost {
    /// Sum of tokens actually processed across every turn. **This is the metric.**
    pub total_prefill_tokens: u64,
    /// Times the prefix had to be discarded and reprocessed.
    pub prefix_invalidations: u32,
    /// Largest context reached. Reported for completeness, deliberately NOT the
    /// objective: optimising this while inflating `total_prefill_tokens` looks
    /// like success and is not.
    pub peak_context_tokens: u32,
    pub turns: u32,
}

/// Append-only assembly across a session.
///
/// On budget overflow the prefix is discarded and the turn starts over from the
/// records it needs — one expensive turn, then cheap ones again.
pub fn simulate_append_only(
    turns: &[Vec<String>],
    cost_of: &dyn Fn(&str) -> u32,
    cfg: AssemblerConfig,
) -> SessionCost {
    let mut prefix = SessionPrefix::new();
    let mut held: u32 = 0;
    let mut out = SessionCost::default();

    for retrieved in turns {
        out.turns += 1;
        let mut plan = plan_append(&prefix, retrieved);
        let mut incoming: u32 = plan.append.iter().map(|d| cost_of(d)).sum();

        if held + incoming > cfg.context_budget_tokens {
            // Eviction: the prefix is gone. Rebuild from just this turn's set.
            out.prefix_invalidations += 1;
            prefix = SessionPrefix::new();
            plan = plan_append(&prefix, retrieved);
            incoming = plan.append.iter().map(|d| cost_of(d)).sum();
            held = 0;
            plan.invalidated_prefix = true;
        }

        // Only the appended tokens are processed; the prefix is reused.
        out.total_prefill_tokens += u64::from(incoming);
        prefix.apply(&plan);
        held += incoming;
        out.peak_context_tokens = out.peak_context_tokens.max(held);
    }
    out
}

/// The comparator: rebuild the whole context from the retrieved set every turn.
///
/// This is what a conventional "assemble the best N tokens per query" retriever
/// does, and it pays full prefill on every turn because the prefix changes.
pub fn simulate_rebuild_per_turn(
    turns: &[Vec<String>],
    cost_of: &dyn Fn(&str) -> u32,
    _cfg: AssemblerConfig,
) -> SessionCost {
    let mut out = SessionCost::default();
    for retrieved in turns {
        out.turns += 1;
        let unique: BTreeSet<&String> = retrieved.iter().collect();
        let n: u32 = unique.into_iter().map(|d| cost_of(d)).sum();
        out.total_prefill_tokens += u64::from(n);
        out.prefix_invalidations += 1; // every turn is a fresh prefix
        out.peak_context_tokens = out.peak_context_tokens.max(n);
    }
    out
}
