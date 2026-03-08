use std::path::Path;
use std::sync::Arc;

use loong_memory_core::{
    Action, AuditSink, DeterministicHashEmbedder, EngineConfig, InMemoryAuditSink, MemoryEngine,
    MemoryPutRequest, PolicyEngine, SqliteStore, StaticPolicy,
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
fn vector_health_reports_invalid_blob_and_non_finite_rows() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("memory.db");
    let ctx = loong_memory_core::OperationContext::new("tester");

    let mut engine_ns1 = build_engine(&db_path, "vh-1");
    let a = engine_ns1
        .put(
            &ctx,
            &MemoryPutRequest {
                namespace: "vh-1".to_owned(),
                external_id: Some("bad-blob".to_owned()),
                content: "bad blob row".to_owned(),
                metadata: json!({}),
            },
        )
        .expect("put row a");
    let b = engine_ns1
        .put(
            &ctx,
            &MemoryPutRequest {
                namespace: "vh-1".to_owned(),
                external_id: Some("nan-blob".to_owned()),
                content: "nan blob row".to_owned(),
                metadata: json!({}),
            },
        )
        .expect("put row b");
    drop(engine_ns1);

    let mut engine_ns2 = build_engine(&db_path, "vh-2");
    let _c = engine_ns2
        .put(
            &ctx,
            &MemoryPutRequest {
                namespace: "vh-2".to_owned(),
                external_id: Some("good".to_owned()),
                content: "healthy row".to_owned(),
                metadata: json!({}),
            },
        )
        .expect("put row c");
    drop(engine_ns2);

    let conn = rusqlite::Connection::open(&db_path).expect("open sqlite raw");
    conn.execute(
        "UPDATE memory_vectors SET vector = ?1 WHERE memory_id = ?2",
        rusqlite::params![vec![1_u8, 2_u8, 3_u8], a.id.as_str()],
    )
    .expect("corrupt blob");
    let nan_blob = vec![f32::NAN; 128]
        .into_iter()
        .flat_map(|v| v.to_le_bytes())
        .collect::<Vec<u8>>();
    conn.execute(
        "UPDATE memory_vectors SET vector = ?1 WHERE memory_id = ?2",
        rusqlite::params![nan_blob, b.id.as_str()],
    )
    .expect("write nan blob");
    drop(conn);

    let store = SqliteStore::open(&db_path).expect("open store for health report");
    let ns1 = store
        .vector_health_report(Some("vh-1"), 10)
        .expect("health report ns1");
    assert_eq!(ns1.total_rows, 2);
    assert_eq!(ns1.blob_rows, 2);
    assert_eq!(ns1.text_rows, 0);
    assert_eq!(ns1.valid_rows, 0);
    assert_eq!(ns1.invalid_rows, 2);
    assert_eq!(ns1.invalid_samples.len(), 2);

    let all = store
        .vector_health_report(None, 10)
        .expect("health report all");
    assert_eq!(all.total_rows, 3);
    assert_eq!(all.valid_rows, 1);
    assert_eq!(all.invalid_rows, 2);
}

#[test]
fn vector_health_counts_text_rows_and_respects_sample_limit() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("memory.db");
    let ctx = loong_memory_core::OperationContext::new("tester");
    let mut engine = build_engine(&db_path, "vh-text");

    let created = engine
        .put(
            &ctx,
            &MemoryPutRequest {
                namespace: "vh-text".to_owned(),
                external_id: Some("legacy-text".to_owned()),
                content: "legacy text row for health".to_owned(),
                metadata: json!({}),
            },
        )
        .expect("put row");
    drop(engine);

    let conn = rusqlite::Connection::open(&db_path).expect("open sqlite raw");
    conn.execute(
        "UPDATE memory_vectors SET vector = ?1 WHERE memory_id = ?2",
        rusqlite::params!["{bad-json", created.id.as_str()],
    )
    .expect("write invalid text vector");
    drop(conn);

    let store = SqliteStore::open(&db_path).expect("open store for health report");
    let report = store
        .vector_health_report(Some("vh-text"), 1)
        .expect("health report");
    assert_eq!(report.total_rows, 1);
    assert_eq!(report.blob_rows, 0);
    assert_eq!(report.text_rows, 1);
    assert_eq!(report.valid_rows, 0);
    assert_eq!(report.invalid_rows, 1);
    assert_eq!(report.invalid_samples.len(), 1);
    assert_eq!(report.invalid_samples[0].sqlite_type, "text");
}
