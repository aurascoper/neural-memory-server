//! The agent-reachable surface: typed MCP tools over JSON-RPC 2.0.
//!
//! This crate owns transport and request validation. It owns **no** domain
//! semantics — every decision about identity, evidence, ranking or retirement is
//! made in `neural-memory-domain` or `neural-memory-store`. Duplicating any of
//! it here would create a second place where the rules could drift.
//!
//! ## What an agent can and cannot reach
//!
//! An earlier draft of the plan listed `record_artifact` and `record_observation`
//! as MCP tools while *also* requiring `Observed` evidence to come from "a
//! channel no MCP tool can reach". Those cannot both be true. The surface is
//! split by reachability instead, and the evidence model wins:
//!
//! | reachable by a model | reachable only by an operator |
//! |---|---|
//! | `recall`, `trace_provenance`, `get_record` | `record-artifact` |
//! | `remember` (always `AgentInference`) | `record-observation` |
//! | `flag_contradiction` | `record-decision`, `supersede` |
//!
//! `supersede` is deliberately out of reach. Retiring a claim is strong and
//! hard to notice, and an agent that can be prompt-injected into retiring true
//! records fails worse than one that cannot retire anything. `flag_contradiction`
//! is the agent's version: it records a `contradicts` edge, which is visible and
//! reviewable and retires nothing.
//!
//! ## No SQL tool, deliberately
//!
//! There is no `query` tool taking SQL. Unrestricted SQL would permit
//! destructive writes, bypass the evidence-class derivation, allow
//! prompt-injection-driven extraction, and let an agent construct unsupported
//! joins and present the result as a fact. Typed tools are the constraint.

pub mod corpus;
pub mod corpus_contested;
pub mod embed;

use neural_memory_domain::{EvidenceClass, MemoryRecordTerms};
use neural_memory_store::{MemoryWrite, RecallOptions, Store, WriteChannel};
use serde_json::{json, Value};

pub const PROTOCOL_VERSION: &str = "2025-06-18";
pub const SERVER_NAME: &str = "neural-memory";
pub const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Session-wide configuration supplied by whoever launched the server.
pub struct Session {
    ///  disables the semantic branch. Retrieval still works; the result
    /// says the branch was absent rather than pretending it ran.
    pub embedder: Option<embed::Embedder>,
    /// Reference instant for recency. Supplied at launch rather than read from
    /// a clock, so retrieval stays reproducible and the store keeps no clock
    /// dependency. A tool call may override it.
    pub as_of: String,
}

/// JSON Schema 2020-12, camelCase, `additionalProperties: false` throughout —
/// the same conventions the upstream `contracts/` directory uses.
pub fn tool_definitions() -> Value {
    json!([
        {
            "name": "recall",
            "description": "Search stored claims by text, expanded along provenance edges. \
                            Retired claims are excluded unless explicitly requested, and any \
                            that were withheld are reported in withheldRetired. \
                            Each hit lists conflictsWith: records it explicitly contradicts. \
                            conflictingPairs lists conflicts where BOTH sides are in these \
                            results — for those the store does not know which is correct, \
                            so say so rather than picking one silently.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {"type": "string", "minLength": 1},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 100, "default": 20},
                    "maxHops": {"type": "integer", "minimum": 0, "maximum": 4, "default": 1},
                    "includeRetired": {"type": "boolean", "default": false},
                    "asOf": {"type": "string", "description":
                        "RFC-3339 instant recency is measured against. Defaults to the \
                         session value fixed at launch; the server never reads a clock."}
                },
                "required": ["query"],
                "additionalProperties": false
            }
        },
        {
            "name": "get_record",
            "description": "Fetch one claim by digest, with the observations it rests on \
                            (each carrying its metric, aggregation and reference execution) \
                            and its provenance edges.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "recordDigest": {"type": "string", "pattern": "^[0-9a-f]{64}$"}
                },
                "required": ["recordDigest"],
                "additionalProperties": false
            }
        },
        {
            "name": "trace_provenance",
            "description": "Walk provenance edges from a record to what it rests on. \
                            Cycle-guarded and depth-bounded.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "recordDigest": {"type": "string", "pattern": "^[0-9a-f]{64}$"},
                    "maxHops": {"type": "integer", "minimum": 1, "maximum": 8, "default": 3}
                },
                "required": ["recordDigest"],
                "additionalProperties": false
            }
        },
        {
            "name": "remember",
            "description": "Record an inference. It is stored as evidenceClass \
                            'agentInference' regardless of what is requested — this tool \
                            cannot produce a higher-trust record. Observed measurements and \
                            human decisions are entered out of band.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "claim": {"type": "string", "minLength": 1},
                    "sourceLocator": {"type": "string"},
                    "occurredAt": {"type": "string"},
                    "supportingObservations": {
                        "type": "array",
                        "items": {"type": "string", "pattern": "^[0-9a-f]{64}$"},
                        "uniqueItems": true
                    }
                },
                "required": ["claim"],
                "additionalProperties": false
            }
        },
        {
            "name": "submit_answer",
            "description": "Submit your final answer. It is CHECKED, not accepted on trust: \
                            if any record you cite has an unresolved conflict with another \
                            stored record, submission is rejected and the conflicts are \
                            returned. You must acknowledge each one in conflictsAcknowledged, \
                            saying which claim you relied on and why, before the answer will \
                            be accepted. Call this exactly once you are ready to answer.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "answer": {"type": "string", "minLength": 1},
                    "citedDigests": {
                        "type": "array",
                        "items": {"type": "string", "pattern": "^[0-9a-f]{64}$"},
                        "uniqueItems": true
                    },
                    "conflictsAcknowledged": {
                        "type": "array",
                        "description": "One entry per conflict you were told about.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "cited": {"type": "string", "pattern": "^[0-9a-f]{64}$"},
                                "conflictsWith": {"type": "string", "pattern": "^[0-9a-f]{64}$"},
                                "resolution": {"type": "string", "minLength": 1}
                            },
                            "required": ["cited", "conflictsWith", "resolution"],
                            "additionalProperties": false
                        }
                    }
                },
                "required": ["answer", "citedDigests"],
                "additionalProperties": false
            }
        },
        {
            "name": "flag_contradiction",
            "description": "Record that two claims appear to conflict. This creates a \
                            reviewable 'contradicts' edge and retires NOTHING. Superseding a \
                            claim is an operator action.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "recordDigest": {"type": "string", "pattern": "^[0-9a-f]{64}$"},
                    "conflictsWith": {"type": "string", "pattern": "^[0-9a-f]{64}$"},
                    "note": {"type": "string"}
                },
                "required": ["recordDigest", "conflictsWith"],
                "additionalProperties": false
            }
        }
    ])
}

#[derive(Debug)]
pub struct ToolError {
    pub message: String,
}

impl ToolError {
    fn new(m: impl Into<String>) -> Self {
        Self { message: m.into() }
    }
}

fn need_str(args: &Value, key: &str) -> Result<String, ToolError> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            ToolError::new(format!(
                "'{key}' is required and must be a non-empty string"
            ))
        })
}

fn digest_arg(args: &Value, key: &str) -> Result<String, ToolError> {
    let v = need_str(args, key)?;
    if !neural_memory_domain::valid_sha256(&v) {
        return Err(ToolError::new(format!(
            "'{key}' must be 64 lowercase hex characters"
        )));
    }
    Ok(v)
}

/// Dispatch one tool call. Separated from stdio so it is testable directly.
pub fn call_tool(
    store: &Store,
    session: &Session,
    name: &str,
    args: &Value,
) -> Result<Value, ToolError> {
    match name {
        "recall" => {
            let query = need_str(args, "query")?;
            let as_of = args
                .get("asOf")
                .and_then(Value::as_str)
                .unwrap_or(&session.as_of)
                .to_string();
            // Embed the query if an embedder is configured AND reachable. A
            // transport failure degrades to lexical + provenance and is
            // reported, rather than failing the whole call: partial retrieval
            // beats none, provided the caller is told which it got.
            let mut sem_vec: Option<Vec<f32>> = None;
            let mut sem_note: Option<String> = None;
            if let Some(e) = &session.embedder {
                match e.embed_query(&query) {
                    Ok(v) => sem_vec = Some(v),
                    Err(err) => sem_note = Some(err.to_string()),
                }
            } else {
                sem_note = Some("no embedder configured; semantic branch absent".into());
            }
            let sem = sem_vec
                .as_ref()
                .map(|v| neural_memory_store::retrieval::SemanticQuery {
                    profile_identity: &session.embedder.as_ref().unwrap().profile_identity,
                    vector: v,
                });
            let opt = RecallOptions {
                query: &query,
                entities: true,
                semantic: sem,
                as_of: &as_of,
                limit: args
                    .get("limit")
                    .and_then(Value::as_u64)
                    .unwrap_or(20)
                    .clamp(1, 100) as usize,
                max_hops: args
                    .get("maxHops")
                    .and_then(Value::as_u64)
                    .unwrap_or(1)
                    .min(4) as u32,
                include_retired: args
                    .get("includeRetired")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            };
            let r = store
                .recall(&opt)
                .map_err(|e| ToolError::new(e.to_string()))?;
            let mut v = serde_json::to_value(r).expect("serialize");
            if let (Some(note), Some(o)) = (sem_note, v.as_object_mut()) {
                o.insert(
                    "semanticBranch".into(),
                    json!({"ran": false, "reason": note}),
                );
            } else if let Some(o) = v.as_object_mut() {
                o.insert("semanticBranch".into(), json!({"ran": true}));
            }
            Ok(v)
        }

        "get_record" => {
            let digest = digest_arg(args, "recordDigest")?;
            match store
                .get_record(&digest)
                .map_err(|e| ToolError::new(e.to_string()))?
            {
                Some(d) => Ok(serde_json::to_value(d).expect("serialize")),
                // Not an error: "no such record" is a legitimate answer, and
                // raising it as a failure invites the caller to retry.
                None => Ok(json!({"found": false, "recordDigest": digest})),
            }
        }

        "trace_provenance" => {
            let digest = digest_arg(args, "recordDigest")?;
            let hops = args
                .get("maxHops")
                .and_then(Value::as_u64)
                .unwrap_or(3)
                .clamp(1, 8) as u32;
            let steps = store
                .trace_provenance(&digest, hops)
                .map_err(|e| ToolError::new(e.to_string()))?;
            Ok(json!({"recordDigest": digest, "steps": steps}))
        }

        "remember" => {
            let claim = need_str(args, "claim")?;
            let observations: Vec<String> = args
                .get("supportingObservations")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            for o in &observations {
                if !neural_memory_domain::valid_sha256(o) {
                    return Err(ToolError::new(format!(
                        "supportingObservations entry {o:?} is not a 64-hex digest"
                    )));
                }
            }
            let w = MemoryWrite {
                terms: MemoryRecordTerms {
                    claim,
                    // Set here for clarity; the store clamps it again on the
                    // Agent channel, before sealing. Neither layer relies on
                    // the other to be the only guard.
                    evidence_class: EvidenceClass::AgentInference,
                    source_artifact_sha256: None,
                    source_locator: args
                        .get("sourceLocator")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    observation_identities: observations,
                    harness_run_id: None,
                },
                occurred_at: args.get("occurredAt").and_then(Value::as_str),
                derivation: None,
            };
            let (digest, wrote) = store
                .put_memory(WriteChannel::Agent, &w)
                .map_err(|e| ToolError::new(e.to_string()))?;
            Ok(json!({
                "recordDigest": digest,
                "evidenceClass": "agentInference",
                "alreadyPresent": !wrote.inserted(),
            }))
        }

        // The store refuses to let a writer declare its own trustworthiness.
        // This applies the same rule one layer up: an agent may not declare
        // that it considered the disagreements in its own evidence. The check
        // is computed from what it cited, so noticing is not optional.
        //
        // H6 arm (c) is why. With conflicts pushed onto every recall hit and
        // named in the tool description, both models still produced confident
        // answers without one word about the disagreement. Visibility was
        // necessary and not sufficient; this is the enforcement.
        "submit_answer" => {
            let answer = need_str(args, "answer")?;
            let cited: Vec<String> = args
                .get("citedDigests")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            if cited.is_empty() {
                return Err(ToolError::new(
                    "citedDigests is empty. An answer that rests on no record cannot be \
                     checked, which is the thing this store exists to replace.",
                ));
            }
            for d in &cited {
                if !neural_memory_domain::valid_sha256(d) {
                    return Err(ToolError::new(format!("{d:?} is not a 64-hex digest")));
                }
                if store
                    .get_record(d)
                    .map_err(|e| ToolError::new(e.to_string()))?
                    .is_none()
                {
                    return Err(ToolError::new(format!("cited record {d} does not exist")));
                }
            }

            let obligations = store
                .conflict_obligations(&cited)
                .map_err(|e| ToolError::new(e.to_string()))?;

            let acked: std::collections::BTreeSet<(String, String)> = args
                .get("conflictsAcknowledged")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|e| {
                            let c = e.get("cited")?.as_str()?.to_string();
                            let w = e.get("conflictsWith")?.as_str()?.to_string();
                            // An empty resolution is not an acknowledgement.
                            e.get("resolution")?
                                .as_str()
                                .filter(|s| !s.trim().is_empty())?;
                            Some((c, w))
                        })
                        .collect()
                })
                .unwrap_or_default();

            let outstanding: Vec<&neural_memory_store::ConflictObligation> = obligations
                .iter()
                .filter(|o| {
                    !acked.contains(&(o.cited.clone(), o.conflicts_with.clone()))
                        && !acked.contains(&(o.conflicts_with.clone(), o.cited.clone()))
                })
                .collect();

            if !outstanding.is_empty() {
                return Ok(json!({
                    "accepted": false,
                    "reason": "unresolved conflicts in the evidence you cited",
                    "instruction": "For each entry below decide which claim you are relying \
                                    on and why, add it to conflictsAcknowledged, and submit \
                                    again. If you cannot tell which is correct, say that in \
                                    the resolution and in your answer.",
                    "unresolvedConflicts": outstanding,
                }));
            }
            Ok(json!({
                "accepted": true,
                "citedCount": cited.len(),
                "conflictsAcknowledged": acked.len(),
                "answerChars": answer.len(),
            }))
        }

        "flag_contradiction" => {
            let a = digest_arg(args, "recordDigest")?;
            let b = digest_arg(args, "conflictsWith")?;
            if a == b {
                return Err(ToolError::new(
                    "a record cannot contradict itself".to_string(),
                ));
            }
            for d in [&a, &b] {
                if store
                    .get_record(d)
                    .map_err(|e| ToolError::new(e.to_string()))?
                    .is_none()
                {
                    return Err(ToolError::new(format!("no such record: {d}")));
                }
            }
            store
                .add_edge(&a, &b, "contradicts", &session.as_of)
                .map_err(|e| ToolError::new(e.to_string()))?;
            Ok(json!({
                "recorded": true,
                "edge": "contradicts",
                "retired": false,
                "note": "Nothing was retired. Superseding a claim is an operator action."
            }))
        }

        other => Err(ToolError::new(format!("unknown tool: {other}"))),
    }
}

/// Handle one JSON-RPC request. Returns `None` for notifications.
pub fn handle_request(store: &Store, session: &Session, req: &Value) -> Option<Value> {
    // A message with no id is a notification and MUST NOT be answered;
    // replying would desynchronise the stream.
    let id = req.get("id").cloned()?;
    let method = req.get("method").and_then(Value::as_str).unwrap_or("");

    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {"tools": {}},
            "serverInfo": {"name": SERVER_NAME, "version": SERVER_VERSION}
        })),
        "tools/list" => Ok(json!({"tools": tool_definitions()})),
        "tools/call" => {
            let params = req.get("params").cloned().unwrap_or(json!({}));
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            match call_tool(store, session, name, &args) {
                Ok(v) => Ok(json!({
                    "content": [{
                        "type": "text",
                        "text": serde_json::to_string_pretty(&v).expect("serialize")
                    }],
                    "isError": false
                })),
                // A tool failure is a RESULT with isError, not a protocol error:
                // the model should see and reason about it rather than have the
                // transport swallow it.
                Err(e) => Ok(json!({
                    "content": [{"type": "text", "text": e.message}],
                    "isError": true
                })),
            }
        }
        "ping" => Ok(json!({})),
        _ => Err((-32601, format!("method not found: {method}"))),
    };

    Some(match result {
        Ok(r) => json!({"jsonrpc": "2.0", "id": id, "result": r}),
        Err((code, message)) => {
            json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
        }
    })
}
