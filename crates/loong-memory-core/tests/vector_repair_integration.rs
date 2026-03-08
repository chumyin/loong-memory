use std::path::Path;
use std::sync::Arc;

use loong_memory_core::{
    Action, AuditSink, DeterministicHashEmbedder, EmbeddingProvider, EngineConfig,
    InMemoryAuditSink, MemoryEngine, MemoryPutRequest, PolicyEngine, SqliteStore, StaticPolicy,
};
use serde_json::json;
use tempfile::tempdir;

fn allowed_policy_for(namespace: &str) -> Arc<dyn PolicyEngine> {
    Arc::new(StaticPolicy::default().allow_namespace_actions(
        namespace.to_owned(),
        [Action::Put, Action::Get, Action::Recall, Action::Delete],
    ))
}

fn build_engine(db_path: &Path, namespace: &str) -> MemoryEngine<SqliteStore> {
    let store = SqliteStore::open(db_path).expect("open sqlite store");
    let policy = allowed_policy_for(namespace);
    let embedder = Arc::new(DeterministicHashEmbedder::new(128));
    let audit: Arc<dyn AuditSink> = Arc::new(InMemoryAuditSink::default());
    MemoryEngine::new(store, policy, embedder, audit, EngineConfig::default())
}

#[test]
fn vector_repair_dry_run_reports_changes_without_writing() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("memory.db");
    let ctx = loong_memory_core::OperationContext::new("tester");
    let mut engine = build_engine(&db_path, "repair");

    let text_row = engine
        .put(
            &ctx,
            &MemoryPutRequest {
                namespace: "repair".to_owned(),
                external_id: Some("text-row".to_owned()),
                content: "repair text row".to_owned(),
                metadata: json!({}),
            },
        )
        .expect("put text row");
    let mismatch_row = engine
        .put(
            &ctx,
            &MemoryPutRequest {
                namespace: "repair".to_owned(),
                external_id: Some("mismatch-row".to_owned()),
                content: "repair mismatch row".to_owned(),
                metadata: json!({}),
            },
        )
        .expect("put mismatch row");
    let bad_row = engine
        .put(
            &ctx,
            &MemoryPutRequest {
                namespace: "repair".to_owned(),
                external_id: Some("bad-row".to_owned()),
                content: "repair bad row".to_owned(),
                metadata: json!({}),
            },
        )
        .expect("put bad row");
    drop(engine);

    let conn = rusqlite::Connection::open(&db_path).expect("open sqlite raw");
    let embedder = DeterministicHashEmbedder::new(128);
    let text_vec = embedder.embed("repair text row").expect("embed text row");
    let text_json = serde_json::to_string(&text_vec).expect("serialize text row");
    conn.execute(
        "UPDATE memory_vectors SET dimension = ?1, vector = ?2 WHERE memory_id = ?3",
        rusqlite::params![7_i64, text_json, text_row.id.as_str()],
    )
    .expect("overwrite text row");
    conn.execute(
        "UPDATE memory_vectors SET dimension = ?1 WHERE memory_id = ?2",
        rusqlite::params![3_i64, mismatch_row.id.as_str()],
    )
    .expect("force mismatch dimension");
    conn.execute(
        "UPDATE memory_vectors SET vector = ?1 WHERE memory_id = ?2",
        rusqlite::params![vec![1_u8, 2_u8, 3_u8], bad_row.id.as_str()],
    )
    .expect("force invalid blob");
    drop(conn);

    let mut store = SqliteStore::open(&db_path).expect("open store");
    let report = store
        .vector_repair(Some("repair"), 20, false)
        .expect("dry-run vector repair");
    assert!(!report.apply);
    assert_eq!(report.total_rows, 3);
    assert_eq!(report.repairable_rows, 2);
    assert_eq!(report.repaired_rows, 0);
    assert_eq!(report.invalid_rows, 1);
    assert_eq!(report.unchanged_rows, 0);
    assert_eq!(report.issues.len(), 1);

    let verify = rusqlite::Connection::open(&db_path).expect("open sqlite verify");
    let text_type: String = verify
        .query_row(
            "SELECT typeof(vector) FROM memory_vectors WHERE memory_id = ?1",
            rusqlite::params![text_row.id.as_str()],
            |row| row.get(0),
        )
        .expect("query text row type");
    assert_eq!(text_type, "text");
    let mismatch_dim: i64 = verify
        .query_row(
            "SELECT dimension FROM memory_vectors WHERE memory_id = ?1",
            rusqlite::params![mismatch_row.id.as_str()],
            |row| row.get(0),
        )
        .expect("query mismatch dimension");
    assert_eq!(mismatch_dim, 3);
}

#[test]
fn vector_repair_apply_converts_text_and_fixes_dimension_mismatch() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("memory.db");
    let ctx = loong_memory_core::OperationContext::new("tester");
    let mut engine = build_engine(&db_path, "repair");

    let text_row = engine
        .put(
            &ctx,
            &MemoryPutRequest {
                namespace: "repair".to_owned(),
                external_id: Some("text-row".to_owned()),
                content: "repair text row".to_owned(),
                metadata: json!({}),
            },
        )
        .expect("put text row");
    let mismatch_row = engine
        .put(
            &ctx,
            &MemoryPutRequest {
                namespace: "repair".to_owned(),
                external_id: Some("mismatch-row".to_owned()),
                content: "repair mismatch row".to_owned(),
                metadata: json!({}),
            },
        )
        .expect("put mismatch row");
    let bad_row = engine
        .put(
            &ctx,
            &MemoryPutRequest {
                namespace: "repair".to_owned(),
                external_id: Some("bad-row".to_owned()),
                content: "repair bad row".to_owned(),
                metadata: json!({}),
            },
        )
        .expect("put bad row");
    drop(engine);

    let conn = rusqlite::Connection::open(&db_path).expect("open sqlite raw");
    let embedder = DeterministicHashEmbedder::new(128);
    let text_vec = embedder.embed("repair text row").expect("embed text row");
    let text_json = serde_json::to_string(&text_vec).expect("serialize text row");
    conn.execute(
        "UPDATE memory_vectors SET dimension = ?1, vector = ?2 WHERE memory_id = ?3",
        rusqlite::params![7_i64, text_json, text_row.id.as_str()],
    )
    .expect("overwrite text row");
    conn.execute(
        "UPDATE memory_vectors SET dimension = ?1 WHERE memory_id = ?2",
        rusqlite::params![3_i64, mismatch_row.id.as_str()],
    )
    .expect("force mismatch dimension");
    conn.execute(
        "UPDATE memory_vectors SET vector = ?1 WHERE memory_id = ?2",
        rusqlite::params![vec![1_u8, 2_u8, 3_u8], bad_row.id.as_str()],
    )
    .expect("force invalid blob");
    drop(conn);

    let mut store = SqliteStore::open(&db_path).expect("open store");
    let report = store
        .vector_repair(Some("repair"), 20, true)
        .expect("apply vector repair");
    assert!(report.apply);
    assert_eq!(report.total_rows, 3);
    assert_eq!(report.repairable_rows, 2);
    assert_eq!(report.repaired_rows, 2);
    assert_eq!(report.invalid_rows, 1);
    assert_eq!(report.unchanged_rows, 0);
    assert_eq!(report.issues.len(), 1);

    let verify = rusqlite::Connection::open(&db_path).expect("open sqlite verify");
    let text_state: (String, i64) = verify
        .query_row(
            "SELECT typeof(vector), dimension FROM memory_vectors WHERE memory_id = ?1",
            rusqlite::params![text_row.id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("query repaired text row");
    assert_eq!(text_state.0, "blob");
    assert_eq!(text_state.1, 128);

    let mismatch_dim: i64 = verify
        .query_row(
            "SELECT dimension FROM memory_vectors WHERE memory_id = ?1",
            rusqlite::params![mismatch_row.id.as_str()],
            |row| row.get(0),
        )
        .expect("query repaired mismatch row");
    assert_eq!(mismatch_dim, 128);

    let bad_type: String = verify
        .query_row(
            "SELECT typeof(vector) FROM memory_vectors WHERE memory_id = ?1",
            rusqlite::params![bad_row.id.as_str()],
            |row| row.get(0),
        )
        .expect("query invalid row type");
    assert_eq!(bad_type, "blob");
}
