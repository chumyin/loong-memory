use std::path::Path;
use std::sync::Arc;

use loong_memory_core::{
    Action, AuditSink, DeterministicHashEmbedder, EmbeddingProvider, EngineConfig,
    InMemoryAuditSink, MemoryEngine, MemoryPutRequest, PolicyEngine, RecallRequest, ScoreWeights,
    SqliteStore, StaticPolicy,
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
fn open_migrates_legacy_text_vectors_to_blob_and_updates_dimension() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("memory.db");
    let ctx = loong_memory_core::OperationContext::new("tester");
    let mut engine = build_engine(&db_path, "migrate");

    let created = engine
        .put(
            &ctx,
            &MemoryPutRequest {
                namespace: "migrate".to_owned(),
                external_id: Some("legacy-json".to_owned()),
                content: "legacy vector migration sample".to_owned(),
                metadata: json!({}),
            },
        )
        .expect("seed put");
    drop(engine);

    let conn = rusqlite::Connection::open(&db_path).expect("open sqlite raw");
    let embedder = DeterministicHashEmbedder::new(128);
    let legacy_json = serde_json::to_string(
        &embedder
            .embed("legacy vector migration sample")
            .expect("embed sample"),
    )
    .expect("serialize legacy json");
    conn.execute(
        "UPDATE memory_vectors SET dimension = ?1, vector = ?2 WHERE memory_id = ?3",
        rusqlite::params![7_i64, legacy_json, created.id.as_str()],
    )
    .expect("overwrite with legacy text vector");
    conn.execute("DELETE FROM schema_migrations WHERE version = 2", [])
        .expect("reset migration marker to simulate pre-v2 database");
    drop(conn);

    let _store = SqliteStore::open(&db_path).expect("reopen triggers migration");

    let verify = rusqlite::Connection::open(&db_path).expect("open verify sqlite");
    let (sqlite_type, dimension): (String, i64) = verify
        .query_row(
            "SELECT typeof(vector), dimension FROM memory_vectors WHERE memory_id = ?1",
            rusqlite::params![created.id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("query migrated vector row");
    assert_eq!(sqlite_type, "blob");
    assert_eq!(dimension, 128);

    let version_count: i64 = verify
        .query_row(
            "SELECT COUNT(*) FROM schema_migrations WHERE version = 2",
            [],
            |row| row.get(0),
        )
        .expect("query migration version");
    assert_eq!(version_count, 1);
}

#[test]
fn open_marks_migration_v2_and_recall_survives_invalid_legacy_text_vector() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("memory.db");
    let ctx = loong_memory_core::OperationContext::new("tester");
    let mut engine = build_engine(&db_path, "migrate");

    let created = engine
        .put(
            &ctx,
            &MemoryPutRequest {
                namespace: "migrate".to_owned(),
                external_id: Some("invalid-legacy".to_owned()),
                content: "invalid legacy text vector should degrade safely".to_owned(),
                metadata: json!({}),
            },
        )
        .expect("seed put");
    drop(engine);

    let conn = rusqlite::Connection::open(&db_path).expect("open sqlite raw");
    conn.execute(
        "UPDATE memory_vectors SET vector = ?1 WHERE memory_id = ?2",
        rusqlite::params!["{not-json", created.id.as_str()],
    )
    .expect("write invalid legacy vector text");
    conn.execute("DELETE FROM schema_migrations WHERE version = 2", [])
        .expect("reset migration marker to simulate pre-v2 database");
    drop(conn);

    let _store = SqliteStore::open(&db_path).expect("reopen triggers migration");
    let verify = rusqlite::Connection::open(&db_path).expect("open verify sqlite");
    let version_count: i64 = verify
        .query_row(
            "SELECT COUNT(*) FROM schema_migrations WHERE version = 2",
            [],
            |row| row.get(0),
        )
        .expect("query migration version");
    assert_eq!(version_count, 1);

    let sqlite_type: String = verify
        .query_row(
            "SELECT typeof(vector) FROM memory_vectors WHERE memory_id = ?1",
            rusqlite::params![created.id.as_str()],
            |row| row.get(0),
        )
        .expect("query invalid row type");
    assert_eq!(sqlite_type, "text");
    drop(verify);

    let engine = build_engine(&db_path, "migrate");
    let hits = engine
        .recall(
            &ctx,
            &RecallRequest {
                namespace: "migrate".to_owned(),
                query: "invalid legacy text vector should degrade safely".to_owned(),
                limit: 1,
                weights: ScoreWeights::default(),
            },
        )
        .expect("recall should survive invalid legacy vector row");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].record.id, created.id);
}
