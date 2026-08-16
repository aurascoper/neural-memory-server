use neural_memory_personal::{
    embeddings::{DeterministicTestEmbedder, PersonalEmbedder},
    personal_mcp::call_tool,
    PersonalStore,
};
use serde_json::{json, Value};

const T0: &str = "2026-08-06T12:00:00.000Z";
const T1: &str = "2026-08-06T12:00:01.000Z";

#[test]
fn personal_tools_bank_search_and_hide_tombstones() {
    let mut store = PersonalStore::open_in_memory().unwrap();
    let remembered = call_tool(
        &mut store,
        "remember",
        &json!({
            "text":"Likes jasmine tea",
            "createdAt":T0,
            "originDevice":"gpd",
            "originRecordID":"local-1",
            "source":"mcp",
            "conversationID":"conversation-1",
            "tags":["preference","tea"]
        }),
    )
    .unwrap();
    let digest = remembered["contentDigest"].as_str().unwrap();

    let found = call_tool(
        &mut store,
        "recall",
        &json!({"query":"jasmine","tag":"tea"}),
    )
    .unwrap();
    let record = &found.as_array().unwrap()[0];
    assert_eq!(record["contentDigest"], digest);
    assert_eq!(record["createdAt"], T0);
    assert_eq!(record["sightings"][0]["capturedAt"], T0);
    assert_eq!(record["sightings"][0]["source"], "mcp");
    assert_eq!(
        record["semanticBranch"],
        json!({"ran":false,"reason":"no-active-profile"})
    );

    assert_eq!(
        call_tool(
            &mut store,
            "forget",
            &json!({"contentDigest":digest,"forgottenAt":T1}),
        )
        .unwrap(),
        json!({"forgotten":true})
    );
    assert_eq!(
        call_tool(&mut store, "list_recent", &json!({}))
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        0
    );
}

#[test]
fn stale_record_is_lexically_visible_with_exact_semantic_unavailability() {
    let mut store = PersonalStore::open_in_memory().unwrap();
    let remembered = call_tool(
        &mut store,
        "remember",
        &json!({
            "text":"Long jasmine memory remains lexically searchable",
            "createdAt":T0,
            "originDevice":"gpd",
            "originRecordID":"long-1"
        }),
    )
    .unwrap();
    let digest = remembered["contentDigest"].as_str().unwrap();
    let embedder = DeterministicTestEmbedder::new(8);
    let profile = store.set_embedding_profile(embedder.profile(), T0).unwrap();
    let pending = call_tool(&mut store, "recall", &json!({"query":"jasmine"})).unwrap();
    assert_eq!(
        pending[0]["semanticBranch"],
        json!({"ran":false,"reason":"pending-local-embedding","profileIdentity":profile})
    );
    assert_eq!(store.rebuild_embeddings(&embedder, 10, T1).unwrap(), 1);
    let ready = call_tool(&mut store, "list_recent", &json!({})).unwrap();
    assert_eq!(
        ready[0]["semanticBranch"],
        json!({"ran":true,"profileIdentity":profile})
    );
    store
        .conn
        .execute(
            "DELETE FROM personal_embeddings WHERE record_digest=?1",
            [digest],
        )
        .unwrap();
    store
        .conn
        .execute(
            "INSERT INTO personal_embedding_failures(profile_identity,record_digest,reason,failed_at) VALUES (?1,?2,'input-too-large',?3)",
            rusqlite::params![profile, digest, T1],
        )
        .unwrap();

    let expected = json!({
        "ran":false,
        "reason":"input-too-large",
        "profileIdentity":profile
    });
    let recall = call_tool(&mut store, "recall", &json!({"query":"jasmine"})).unwrap();
    assert_eq!(
        recall[0]["text"],
        "Long jasmine memory remains lexically searchable"
    );
    assert_eq!(recall[0]["semanticBranch"], expected);
    let recent = call_tool(&mut store, "list_recent", &json!({})).unwrap();
    assert_eq!(recent[0]["semanticBranch"], expected);
    let context = store.local_context("jasmine", 10).unwrap();
    assert_eq!(
        context["records"][0]["text"],
        "Long jasmine memory remains lexically searchable"
    );
    assert_eq!(context["records"][0]["semanticBranch"], expected);

    let failure_columns: Vec<String> = store
        .conn
        .prepare("PRAGMA table_info(personal_embedding_failures)")
        .unwrap()
        .query_map([], |row| row.get(1))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(!failure_columns.iter().any(|column| column == "text"));
}

#[test]
fn personal_tool_surface_is_strict() {
    let mut store = PersonalStore::open_in_memory().unwrap();
    for forbidden in ["query", "sql", "evidence_search", "filesystem"] {
        assert!(call_tool(&mut store, forbidden, &Value::Null).is_err());
    }
    assert!(call_tool(
        &mut store,
        "forget",
        &json!({"contentDigest":"A".repeat(64),"forgottenAt":T0})
    )
    .is_err());
    assert!(call_tool(
        &mut store,
        "remember",
        &json!({"text":"x","createdAt":T0,"originDevice":"gpd","originRecordID":"1","tags":[1]})
    )
    .is_err());
}

#[test]
fn unacknowledged_divergence_blocks_both_records_from_personal_reads() {
    let mut store = PersonalStore::open_in_memory().unwrap();
    let a = call_tool(
        &mut store,
        "remember",
        &json!({"text":"Tea is preferred","createdAt":T0,"originDevice":"gpd","originRecordID":"div-a"}),
    )
    .unwrap()["contentDigest"]
        .as_str()
        .unwrap()
        .to_string();
    let b = call_tool(
        &mut store,
        "remember",
        &json!({"text":"Tea is avoided","createdAt":T1,"originDevice":"gpd","originRecordID":"div-b"}),
    )
    .unwrap()["contentDigest"]
        .as_str()
        .unwrap()
        .to_string();
    let divergence = store.flag_divergence(&a, &b, T1).unwrap();

    assert!(call_tool(&mut store, "recall", &json!({"query":"Tea"}))
        .unwrap()
        .as_array()
        .unwrap()
        .is_empty());
    assert!(call_tool(&mut store, "list_recent", &json!({}))
        .unwrap()
        .as_array()
        .unwrap()
        .is_empty());
    assert!(store.local_context("Tea", 10).unwrap()["records"]
        .as_array()
        .unwrap()
        .is_empty());
    let status = store
        .embedding_status(std::path::Path::new("personal.db"))
        .unwrap();
    assert_eq!(status.unacknowledged_divergences, 1);
    assert_eq!(status.health, "blocked:unacknowledged-divergence");

    assert!(store.acknowledge_divergence(&divergence.id, T1).unwrap());
    assert_eq!(
        call_tool(&mut store, "recall", &json!({"query":"Tea"}))
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        call_tool(&mut store, "list_recent", &json!({}))
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        store.local_context("Tea", 10).unwrap()["records"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}
