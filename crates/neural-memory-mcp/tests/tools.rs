//! MCP surface: protocol, tool behaviour, and the reachability boundary.

use neural_memory_domain::*;
use neural_memory_mcp::*;
use neural_memory_store::*;
use serde_json::{json, Value};

const NOW: &str = "2026-07-30T12:00:00Z";

fn session() -> Session {
    Session {
        as_of: NOW.to_string(),
        embedder: None,
    }
}

fn claim(s: &Store, text: &str) -> String {
    let w = MemoryWrite {
        terms: MemoryRecordTerms {
            claim: text.into(),
            evidence_class: EvidenceClass::ExternalClaim,
            source_artifact_sha256: None,
            source_locator: None,
            observation_identities: vec![],
            harness_run_id: None,
        },
        occurred_at: None,
        derivation: None,
    };
    s.put_memory(WriteChannel::Operator, &w).unwrap().0
}

/// Parse the JSON payload back out of an MCP tool result envelope.
fn payload(resp: &Value) -> Value {
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    serde_json::from_str(text).unwrap()
}

fn call(store: &Store, name: &str, args: Value) -> Value {
    handle_request(
        store,
        &session(),
        &json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
                "params":{"name":name,"arguments":args}}),
    )
    .unwrap()
}

// ---------------------------------------------------------------------------
// Protocol
// ---------------------------------------------------------------------------

#[test]
fn initialize_reports_protocol_and_server_identity() {
    let s = Store::open_in_memory().unwrap();
    let r = handle_request(
        &s,
        &session(),
        &json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
    )
    .unwrap();
    assert_eq!(r["jsonrpc"], "2.0");
    assert_eq!(r["id"], 1);
    assert_eq!(r["result"]["protocolVersion"], PROTOCOL_VERSION);
    assert_eq!(r["result"]["serverInfo"]["name"], SERVER_NAME);
    assert!(r["result"]["capabilities"]["tools"].is_object());
}

#[test]
fn a_notification_gets_no_response() {
    // JSON-RPC: a message with no id is a notification and MUST NOT be answered.
    // Replying would desynchronise the stream.
    let s = Store::open_in_memory().unwrap();
    assert!(handle_request(
        &s,
        &session(),
        &json!({"jsonrpc":"2.0","method":"notifications/initialized"})
    )
    .is_none());
}

#[test]
fn an_unknown_method_is_a_protocol_error() {
    let s = Store::open_in_memory().unwrap();
    let r = handle_request(
        &s,
        &session(),
        &json!({"jsonrpc":"2.0","id":7,"method":"tools/delete"}),
    )
    .unwrap();
    assert_eq!(r["error"]["code"], -32601);
    assert!(r.get("result").is_none());
}

#[test]
fn a_tool_failure_is_a_result_with_is_error_not_a_protocol_error() {
    // The model must see and reason about a tool failure; a protocol error
    // would have the transport swallow it.
    let s = Store::open_in_memory().unwrap();
    let r = call(&s, "get_record", json!({"recordDigest": "not-a-digest"}));
    assert!(r.get("error").is_none(), "must not be a JSON-RPC error");
    assert_eq!(r["result"]["isError"], true);
    assert!(r["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("64 lowercase hex"));
}

#[test]
fn every_advertised_tool_is_dispatchable_and_schema_shaped() {
    let s = Store::open_in_memory().unwrap();
    let defs = tool_definitions();
    let names: Vec<&str> = defs
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        vec![
            "recall",
            "get_record",
            "trace_provenance",
            "remember",
            "submit_answer",
            "flag_contradiction"
        ]
    );

    for t in defs.as_array().unwrap() {
        let schema = &t["inputSchema"];
        assert_eq!(schema["type"], "object");
        assert_eq!(
            schema["additionalProperties"], false,
            "{}: unknown arguments must be rejected, not ignored",
            t["name"]
        );
        assert!(schema["required"].is_array());
        // Dispatchable: calling with empty args must fail as a TOOL error, not
        // an unknown-tool error.
        let r = call(&s, t["name"].as_str().unwrap(), json!({}));
        let text = r["result"]["content"][0]["text"].as_str().unwrap();
        assert!(
            !text.contains("unknown tool"),
            "{} is advertised but not dispatchable",
            t["name"]
        );
    }
}

// ---------------------------------------------------------------------------
// The reachability boundary
// ---------------------------------------------------------------------------

#[test]
fn there_is_no_tool_that_takes_sql() {
    // Unrestricted SQL would permit destructive writes, bypass evidence-class
    // derivation, enable prompt-injection-driven extraction, and let an agent
    // build unsupported joins and present them as facts.
    let defs = tool_definitions();
    let blob = serde_json::to_string(&defs).unwrap().to_lowercase();
    for banned in ["\"sql\"", "\"query_raw\"", "\"execute\"", "\"statement\""] {
        assert!(
            !blob.contains(banned),
            "{banned} must not appear in the tool surface"
        );
    }
    let s = Store::open_in_memory().unwrap();
    let r = call(&s, "sql", json!({"sql": "DROP TABLE memories"}));
    assert!(r["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("unknown tool"));
}

#[test]
fn no_write_side_tool_is_reachable_except_remember_and_flag() {
    // The evidence model requires `Observed` to rest on an artifact ingested
    // through a channel no MCP tool can reach. If any of these were dispatchable
    // that requirement would be satisfiable by the agent itself.
    let s = Store::open_in_memory().unwrap();
    for forbidden in [
        "record_artifact",
        "record_observation",
        "record_decision",
        "supersede_claim",
        "supersede",
    ] {
        let r = call(&s, forbidden, json!({}));
        assert!(
            r["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("unknown tool"),
            "{forbidden} must not be reachable from the MCP surface"
        );
    }
}

#[test]
fn remember_always_produces_agent_inference() {
    let s = Store::open_in_memory().unwrap();
    let r = call(
        &s,
        "remember",
        json!({"claim": "Vulkan appears slower on battery"}),
    );
    let p = payload(&r);
    assert_eq!(p["evidenceClass"], "agentInference");

    let class: String = s
        .conn
        .query_row(
            "SELECT evidence_class FROM memories WHERE record_digest = ?1",
            [p["recordDigest"].as_str().unwrap()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(class, "agentInference");

    // There is no argument through which a different class could be requested.
    let schema = &tool_definitions()[3]["inputSchema"]["properties"];
    assert!(schema.get("evidenceClass").is_none());
}

#[test]
fn remember_is_idempotent_and_says_so() {
    let s = Store::open_in_memory().unwrap();
    let a = payload(&call(&s, "remember", json!({"claim": "same claim"})));
    let b = payload(&call(&s, "remember", json!({"claim": "same claim"})));
    assert_eq!(a["recordDigest"], b["recordDigest"]);
    assert_eq!(a["alreadyPresent"], false);
    assert_eq!(b["alreadyPresent"], true);
}

#[test]
fn flag_contradiction_records_an_edge_and_retires_nothing() {
    let s = Store::open_in_memory().unwrap();
    let a = claim(&s, "14B pp512 is 159.96");
    let b = claim(&s, "14B pp512 is 147.91");

    let p = payload(&call(
        &s,
        "flag_contradiction",
        json!({"recordDigest": a, "conflictsWith": b}),
    ));
    assert_eq!(p["retired"], false);

    // Neither record was retired, and both are still returned by ordinary recall.
    for d in [&a, &b] {
        let sup: Option<String> = s
            .conn
            .query_row(
                "SELECT superseded_at FROM memories WHERE record_digest = ?1",
                [d],
                |r| r.get(0),
            )
            .unwrap();
        assert!(sup.is_none(), "flag_contradiction must not retire anything");
    }
    let edges: i64 = s
        .conn
        .query_row(
            "SELECT count(*) FROM provenance_edges WHERE edge_kind = 'contradicts'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(edges, 1);
}

#[test]
fn flag_contradiction_rejects_unknown_records_and_self_reference() {
    let s = Store::open_in_memory().unwrap();
    let a = claim(&s, "a claim");

    let r = call(
        &s,
        "flag_contradiction",
        json!({"recordDigest": a, "conflictsWith": a}),
    );
    assert_eq!(r["result"]["isError"], true);

    let r = call(
        &s,
        "flag_contradiction",
        json!({"recordDigest": a, "conflictsWith": "ab".repeat(32)}),
    );
    assert!(r["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("no such record"));
}

// ---------------------------------------------------------------------------
// Read tools
// ---------------------------------------------------------------------------

#[test]
fn recall_returns_hits_counts_and_withheld() {
    let s = Store::open_in_memory().unwrap();
    let old = claim(&s, "Qwen3 14B pp512 is 159.96 tokens per second");
    let new = claim(&s, "Qwen3 14B pp512 is 147.91 tokens per second");
    s.supersede(&old, &new, NOW).unwrap();

    let p = payload(&call(&s, "recall", json!({"query": "14B pp512"})));
    let hits = p["hits"].as_array().unwrap();
    assert!(hits.iter().all(|h| h["recordDigest"] != old.as_str()));
    assert!(p["withheldRetired"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d == &json!(old)));
    assert!(p["counts"]["unique"].as_u64().unwrap() >= 1);
}

#[test]
fn recall_uses_the_session_as_of_when_none_is_given() {
    // The server never reads a clock; the reference instant comes from launch.
    let s = Store::open_in_memory().unwrap();
    let w = MemoryWrite {
        terms: MemoryRecordTerms {
            claim: "divergence measured".into(),
            evidence_class: EvidenceClass::ExternalClaim,
            source_artifact_sha256: None,
            source_locator: None,
            observation_identities: vec![],
            harness_run_id: None,
        },
        occurred_at: Some("2026-01-01T00:00:00Z"),
        derivation: None,
    };
    s.put_memory(WriteChannel::Operator, &w).unwrap();

    let default = payload(&call(&s, "recall", json!({"query": "divergence"})));
    let overridden = payload(&call(
        &s,
        "recall",
        json!({"query": "divergence", "asOf": "2029-01-01T00:00:00Z"}),
    ));
    let score = |p: &Value| p["hits"][0]["recencyScore"].as_f64().unwrap();
    assert!(
        score(&default) > score(&overridden),
        "an explicit asOf must override the session value"
    );
}

#[test]
fn get_record_returns_observations_with_their_metric_and_reference() {
    // A cited number must arrive with what it was measured by and against --
    // that is the whole point of storing it rather than quoting prose.
    let s = Store::open_in_memory().unwrap();
    let (suite, _) = s
        .put_evaluation_suite(&EvaluationSuiteTerms {
            suite_name: "gpd-single-prompt-greedy".into(),
            case_digests: vec!["bb".repeat(32)],
            tokenizer_identity: "gemma4-tok".into(),
            context_cap: 8192,
        })
        .unwrap();
    let (pol, _) = s
        .put_measurement_policy(&MeasurementPolicyTerms {
            metric: "maxAbsoluteLogitDelta".into(),
            aggregation: "maxOverPreDivergenceSteps".into(),
            comparison_rule: "lessThanOrEqualTolerance".into(),
            step_budget: Some(58),
            unit: "logit".into(),
        })
        .unwrap();
    s.put_artifact(
        &ArtifactTerms {
            artifact_kind: "gguf".into(),
            sha256_hex: "cc".repeat(32),
            byte_size: 1,
            media_type: "application/octet-stream".into(),
            source_uri: "file:///m.gguf".into(),
        },
        NOW,
    )
    .unwrap();
    let (refx, _) = s
        .put_reference_execution(&ReferenceExecutionTerms {
            runtime_identity: "llama.cpp-b10188".into(),
            backend_id: "llama-cpp-cpu".into(),
            artifact_sha256: "cc".repeat(32),
            evaluation_suite_identity: suite.clone(),
            environment: vec!["os=ubuntu-26.04".into()],
        })
        .unwrap();
    let (obs, _) = s
        .put_observation(
            &ObservationTerms {
                observation_kind: "maxLogitDivergence".into(),
                quantity_kind: QuantityKind::Relative,
                value_text: "4.3362".into(),
                measurement_policy_identity: pol,
                evaluation_suite_identity: suite,
                reference_execution_identity: Some(refx),
                runtime_identity: "llama.cpp-b10188".into(),
                artifact_sha256: None,
            },
            NOW,
        )
        .unwrap();

    let w = MemoryWrite {
        terms: MemoryRecordTerms {
            claim: "Gemma Vulkan exceeded the tolerance".into(),
            evidence_class: EvidenceClass::ExternalClaim,
            source_artifact_sha256: None,
            source_locator: Some("§4".into()),
            observation_identities: vec![obs],
            harness_run_id: None,
        },
        occurred_at: None,
        derivation: None,
    };
    let (digest, _) = s.put_memory(WriteChannel::Operator, &w).unwrap();

    let p = payload(&call(&s, "get_record", json!({"recordDigest": digest})));
    let o = &p["observations"][0];
    assert_eq!(o["valueText"], "4.3362");
    assert_eq!(o["metric"], "maxAbsoluteLogitDelta");
    assert_eq!(o["aggregation"], "maxOverPreDivergenceSteps");
    assert_eq!(o["stepBudget"], 58);
    assert_eq!(o["referenceBackend"], "llama-cpp-cpu");
    assert!(
        o["referenceExecution"].is_string(),
        "a relative quantity always arrives with its referent"
    );
}

#[test]
fn get_record_reports_a_miss_as_data_not_as_an_error() {
    // "No such record" is a legitimate answer. Raising it as a failure invites
    // the caller to retry a query that will never succeed.
    let s = Store::open_in_memory().unwrap();
    let r = call(&s, "get_record", json!({"recordDigest": "ab".repeat(32)}));
    assert_eq!(r["result"]["isError"], false);
    assert_eq!(payload(&r)["found"], false);
}

#[test]
fn trace_provenance_returns_the_chain_with_depths() {
    let s = Store::open_in_memory().unwrap();
    let top = claim(&s, "Vulkan requires its own numerical contract");
    let mid = claim(&s, "max logit delta was 4.3362");
    let base = claim(&s, "the CPU reference execution");
    s.add_edge(&top, &mid, "derivedFrom", NOW).unwrap();
    s.add_edge(&mid, &base, "derivedFrom", NOW).unwrap();

    let p = payload(&call(
        &s,
        "trace_provenance",
        json!({"recordDigest": top, "maxHops": 3}),
    ));
    let steps = p["steps"].as_array().unwrap();
    assert_eq!(steps.len(), 2);
    assert_eq!(steps[0]["hops"], 1);
    assert_eq!(steps[1]["hops"], 2);
}

#[test]
fn tool_arguments_are_bounded_rather_than_trusted() {
    let s = Store::open_in_memory().unwrap();
    for _ in 0..5 {
        claim(&s, "divergence note");
    }
    // An out-of-range limit is clamped, not honoured and not rejected.
    let p = payload(&call(
        &s,
        "recall",
        json!({"query": "divergence", "limit": 100000, "maxHops": 99}),
    ));
    assert!(p["hits"].as_array().unwrap().len() <= 100);
}

// ---------------------------------------------------------------------------
// submit_answer -- the obligation is checked, not requested
// ---------------------------------------------------------------------------

fn conflicting_pair(s: &Store) -> (String, String) {
    let a = claim(
        s,
        "Vulkan is blocked because the policy has no backend dimension",
    );
    let b = claim(
        s,
        "Vulkan is blocked by the measurement corpus, not the schema",
    );
    s.add_edge(&a, &b, "contradicts", NOW).unwrap();
    (a, b)
}

#[test]
fn submitting_an_answer_over_conflicted_evidence_is_rejected() {
    // H6 arm (c): with conflicts pushed onto every hit AND named in the tool
    // description, both models still answered confidently without one word
    // about the disagreement. Visibility was necessary and not sufficient.
    let s = Store::open_in_memory().unwrap();
    let (a, _b) = conflicting_pair(&s);

    let p = payload(&call(
        &s,
        "submit_answer",
        json!({
            "answer": "Vulkan is blocked because the policy has no backend dimension.",
            "citedDigests": [a],
        }),
    ));
    assert_eq!(p["accepted"], false);
    assert_eq!(p["unresolvedConflicts"].as_array().unwrap().len(), 1);
    // The rejection names the rival claim, so the agent does not have to guess
    // what it failed to consider.
    assert!(p["unresolvedConflicts"][0]["conflictingClaim"]
        .as_str()
        .unwrap()
        .contains("measurement corpus"));
}

#[test]
fn acknowledging_the_conflict_lets_the_answer_through() {
    // Polarity: a gate that rejected everything would pass the test above.
    let s = Store::open_in_memory().unwrap();
    let (a, b) = conflicting_pair(&s);

    let p = payload(&call(
        &s,
        "submit_answer",
        json!({
            "answer": "Blocked by the measurement corpus.",
            "citedDigests": [a.clone()],
            "conflictsAcknowledged": [{
                "cited": a, "conflictsWith": b,
                "resolution": "The backend-dimension account is the older reading; I relied on the corpus one."
            }],
        }),
    ));
    assert_eq!(p["accepted"], true);
    assert_eq!(p["conflictsAcknowledged"], 1);
}

#[test]
fn an_empty_resolution_is_not_an_acknowledgement() {
    // Otherwise the gate is satisfiable by pasting the digests back with no
    // thought at all -- an obligation in form only.
    let s = Store::open_in_memory().unwrap();
    let (a, b) = conflicting_pair(&s);
    let p = payload(&call(
        &s,
        "submit_answer",
        json!({
            "answer": "x", "citedDigests": [a.clone()],
            "conflictsAcknowledged": [{"cited": a, "conflictsWith": b, "resolution": "   "}],
        }),
    ));
    assert_eq!(p["accepted"], false);
}

#[test]
fn acknowledgement_is_symmetric() {
    // The agent should not have to guess which way round the edge was written.
    let s = Store::open_in_memory().unwrap();
    let (a, b) = conflicting_pair(&s);
    let p = payload(&call(
        &s,
        "submit_answer",
        json!({
            "answer": "x", "citedDigests": [a.clone()],
            "conflictsAcknowledged": [{"cited": b, "conflictsWith": a, "resolution": "considered"}],
        }),
    ));
    assert_eq!(p["accepted"], true);
}

#[test]
fn a_retired_counterpart_raises_no_obligation() {
    // Where the store knows which claim is current there is nothing to
    // adjudicate, and demanding an acknowledgement would train the agent to
    // rubber-stamp them.
    let s = Store::open_in_memory().unwrap();
    let (a, b) = conflicting_pair(&s);
    s.supersede(&a, &b, NOW).unwrap();

    let p = payload(&call(
        &s,
        "submit_answer",
        json!({
            "answer": "Blocked by the measurement corpus.", "citedDigests": [b],
        }),
    ));
    assert_eq!(p["accepted"], true, "retirement already resolves it");
}

#[test]
fn an_answer_citing_nothing_is_rejected() {
    let s = Store::open_in_memory().unwrap();
    claim(&s, "some claim");
    let r = call(
        &s,
        "submit_answer",
        json!({"answer": "trust me", "citedDigests": []}),
    );
    assert_eq!(r["result"]["isError"], true);
}

#[test]
fn citing_a_record_that_does_not_exist_is_rejected() {
    let s = Store::open_in_memory().unwrap();
    let r = call(
        &s,
        "submit_answer",
        json!({"answer": "x", "citedDigests": ["ab".repeat(32)]}),
    );
    assert_eq!(r["result"]["isError"], true);
    assert!(r["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("does not exist"));
}
