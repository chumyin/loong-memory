use std::sync::Arc;

use loong_memory_core::{
    Action, AllowAllPolicy, AuditEventKind, AuditSink, DeterministicHashEmbedder,
    EmbeddingProvider, EngineConfig, InMemoryAuditSink, LoongMemoryError, MemoryDeleteRequest,
    MemoryEngine, MemoryGetRequest, MemoryPutRequest, RecallRequest, ScoreWeights, SqliteStore,
    StaticPolicy,
};
use serde_json::json;
use tempfile::tempdir;

fn allowed_policy_for(namespace: &str) -> Arc<dyn loong_memory_core::PolicyEngine> {
    Arc::new(StaticPolicy::default().allow_namespace_actions(
        namespace.to_owned(),
        [
            Action::Put,
            Action::Get,
            Action::Recall,
            Action::Delete,
            Action::AuditRead,
        ],
    ))
}

fn build_engine(
    db_path: &std::path::Path,
    policy: Arc<dyn loong_memory_core::PolicyEngine>,
    audit: Arc<InMemoryAuditSink>,
) -> MemoryEngine<SqliteStore> {
    let store = SqliteStore::open(db_path).expect("open sqlite");
    let embedder = Arc::new(DeterministicHashEmbedder::new(128));
    let audit_sink: Arc<dyn AuditSink> = audit;
    MemoryEngine::new(store, policy, embedder, audit_sink, EngineConfig::default())
}

#[test]
fn put_get_delete_roundtrip_by_external_id_and_id() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("memory.db");
    let audit = Arc::new(InMemoryAuditSink::default());
    let mut engine = build_engine(&db_path, allowed_policy_for("agent-a"), audit);
    let ctx = loong_memory_core::OperationContext::new("tester");

    let created = engine
        .put(
            &ctx,
            &MemoryPutRequest {
                namespace: "agent-a".to_owned(),
                external_id: Some("profile".to_owned()),
                content: "Alice likes rust systems programming".to_owned(),
                metadata: json!({"source":"seed"}),
            },
        )
        .expect("put created");

    let fetched_by_external = engine
        .get(
            &ctx,
            &MemoryGetRequest {
                namespace: "agent-a".to_owned(),
                id: None,
                external_id: Some("profile".to_owned()),
            },
        )
        .expect("get external");
    assert_eq!(created.id, fetched_by_external.id);

    let updated = engine
        .put(
            &ctx,
            &MemoryPutRequest {
                namespace: "agent-a".to_owned(),
                external_id: Some("profile".to_owned()),
                content: "Alice likes rust and sqlite internals".to_owned(),
                metadata: json!({"source":"seed","rev":2}),
            },
        )
        .expect("put update");
    assert_eq!(created.id, updated.id);

    let fetched_by_id = engine
        .get(
            &ctx,
            &MemoryGetRequest {
                namespace: "agent-a".to_owned(),
                id: Some(updated.id.clone()),
                external_id: None,
            },
        )
        .expect("get id");
    assert_eq!(
        fetched_by_id.content,
        "Alice likes rust and sqlite internals"
    );

    engine
        .delete(
            &ctx,
            &MemoryDeleteRequest {
                namespace: "agent-a".to_owned(),
                id: None,
                external_id: Some("profile".to_owned()),
            },
        )
        .expect("delete");

    let err = engine
        .get(
            &ctx,
            &MemoryGetRequest {
                namespace: "agent-a".to_owned(),
                id: Some(updated.id),
                external_id: None,
            },
        )
        .expect_err("not found after delete");
    assert!(matches!(err, LoongMemoryError::NotFound));
}

#[test]
fn namespace_isolation_prevents_cross_namespace_read() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("memory.db");
    let audit = Arc::new(InMemoryAuditSink::default());
    let policy = Arc::new(AllowAllPolicy);
    let mut engine = build_engine(&db_path, policy, audit);
    let ctx = loong_memory_core::OperationContext::new("tester");

    let created = engine
        .put(
            &ctx,
            &MemoryPutRequest {
                namespace: "alpha".to_owned(),
                external_id: None,
                content: "alpha-only secret".to_owned(),
                metadata: json!({"scope":"alpha"}),
            },
        )
        .expect("put");

    let err = engine
        .get(
            &ctx,
            &MemoryGetRequest {
                namespace: "beta".to_owned(),
                id: Some(created.id),
                external_id: None,
            },
        )
        .expect_err("cross namespace should fail");
    assert!(matches!(err, LoongMemoryError::NotFound));
}

#[test]
fn policy_deny_blocks_write_and_emits_denied_audit() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("memory.db");
    let audit = Arc::new(InMemoryAuditSink::default());
    let policy = Arc::new(StaticPolicy::default());
    let mut engine = build_engine(&db_path, policy, Arc::clone(&audit));
    let ctx = loong_memory_core::OperationContext::new("restricted-user");

    let err = engine
        .put(
            &ctx,
            &MemoryPutRequest {
                namespace: "secure".to_owned(),
                external_id: None,
                content: "should be blocked".to_owned(),
                metadata: json!({}),
            },
        )
        .expect_err("policy should deny");
    assert!(matches!(err, LoongMemoryError::PolicyDenied(_)));

    let events = audit.snapshot();
    assert!(
        events
            .iter()
            .any(|evt| matches!(evt.kind, AuditEventKind::Denied)),
        "expected at least one denied audit event"
    );
}

#[test]
fn recall_returns_relevant_results_with_hybrid_scoring() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("memory.db");
    let audit = Arc::new(InMemoryAuditSink::default());
    let mut engine = build_engine(&db_path, allowed_policy_for("agent-r"), audit);
    let ctx = loong_memory_core::OperationContext::new("tester");

    let records = [
        "apple banana nutrition facts",
        "tcp socket timeout tuning guide",
        "apple pie recipe with cinnamon",
    ];
    for content in records {
        engine
            .put(
                &ctx,
                &MemoryPutRequest {
                    namespace: "agent-r".to_owned(),
                    external_id: None,
                    content: content.to_owned(),
                    metadata: json!({}),
                },
            )
            .expect("seed put");
    }

    let hits = engine
        .recall(
            &ctx,
            &RecallRequest {
                namespace: "agent-r".to_owned(),
                query: "apple nutrition".to_owned(),
                limit: 2,
                weights: ScoreWeights {
                    lexical: 0.6,
                    vector: 0.4,
                },
            },
        )
        .expect("recall");

    assert_eq!(hits.len(), 2);
    assert!(
        hits[0].record.content.contains("apple"),
        "top hit should be apple-related"
    );
    assert!(hits[0].hybrid_score >= hits[1].hybrid_score);
}

#[test]
fn audit_contains_allow_and_operation_events() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("memory.db");
    let audit = Arc::new(InMemoryAuditSink::default());
    let mut engine = build_engine(&db_path, allowed_policy_for("ns-audit"), Arc::clone(&audit));
    let ctx = loong_memory_core::OperationContext::new("auditor");

    let created = engine
        .put(
            &ctx,
            &MemoryPutRequest {
                namespace: "ns-audit".to_owned(),
                external_id: Some("k1".to_owned()),
                content: "audit me".to_owned(),
                metadata: json!({}),
            },
        )
        .expect("put");
    let _ = engine
        .get(
            &ctx,
            &MemoryGetRequest {
                namespace: "ns-audit".to_owned(),
                id: Some(created.id.clone()),
                external_id: None,
            },
        )
        .expect("get");
    let _ = engine
        .recall(
            &ctx,
            &RecallRequest {
                namespace: "ns-audit".to_owned(),
                query: "audit".to_owned(),
                limit: 1,
                weights: ScoreWeights::default(),
            },
        )
        .expect("recall");
    engine
        .delete(
            &ctx,
            &MemoryDeleteRequest {
                namespace: "ns-audit".to_owned(),
                id: Some(created.id),
                external_id: None,
            },
        )
        .expect("delete");

    let events = audit.snapshot();
    assert!(events
        .iter()
        .any(|evt| matches!(evt.kind, AuditEventKind::Allowed)));
    assert!(events
        .iter()
        .any(|evt| matches!(evt.kind, AuditEventKind::Write)));
    assert!(events
        .iter()
        .any(|evt| matches!(evt.kind, AuditEventKind::Read)));
    assert!(events
        .iter()
        .any(|evt| matches!(evt.kind, AuditEventKind::Recall)));
    assert!(events
        .iter()
        .any(|evt| matches!(evt.kind, AuditEventKind::Delete)));
}

#[test]
fn selector_validation_rejects_id_and_external_id_together() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("memory.db");
    let audit = Arc::new(InMemoryAuditSink::default());
    let mut engine = build_engine(
        &db_path,
        allowed_policy_for("ns-validation"),
        Arc::clone(&audit),
    );
    let ctx = loong_memory_core::OperationContext::new("validator");

    let _created = engine
        .put(
            &ctx,
            &MemoryPutRequest {
                namespace: "ns-validation".to_owned(),
                external_id: Some("e1".to_owned()),
                content: "payload".to_owned(),
                metadata: json!({}),
            },
        )
        .expect("seed put");

    let get_err = engine
        .get(
            &ctx,
            &MemoryGetRequest {
                namespace: "ns-validation".to_owned(),
                id: Some("x".to_owned()),
                external_id: Some("e1".to_owned()),
            },
        )
        .expect_err("get selector should be invalid");
    assert!(matches!(get_err, LoongMemoryError::Validation(_)));

    let delete_err = engine
        .delete(
            &ctx,
            &MemoryDeleteRequest {
                namespace: "ns-validation".to_owned(),
                id: Some("x".to_owned()),
                external_id: Some("e1".to_owned()),
            },
        )
        .expect_err("delete selector should be invalid");
    assert!(matches!(delete_err, LoongMemoryError::Validation(_)));
}

#[test]
fn recall_validation_rejects_invalid_weights() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("memory.db");
    let audit = Arc::new(InMemoryAuditSink::default());
    let mut engine = build_engine(&db_path, allowed_policy_for("ns-weight"), audit);
    let ctx = loong_memory_core::OperationContext::new("validator");

    engine
        .put(
            &ctx,
            &MemoryPutRequest {
                namespace: "ns-weight".to_owned(),
                external_id: None,
                content: "hello".to_owned(),
                metadata: json!({}),
            },
        )
        .expect("seed");

    let err = engine
        .recall(
            &ctx,
            &RecallRequest {
                namespace: "ns-weight".to_owned(),
                query: "hello".to_owned(),
                limit: 1,
                weights: ScoreWeights {
                    lexical: -1.0,
                    vector: 1.0,
                },
            },
        )
        .expect_err("negative weight should be invalid");
    assert!(matches!(err, LoongMemoryError::Validation(_)));

    let err = engine
        .recall(
            &ctx,
            &RecallRequest {
                namespace: "ns-weight".to_owned(),
                query: "hello".to_owned(),
                limit: 1,
                weights: ScoreWeights {
                    lexical: 0.0,
                    vector: 0.0,
                },
            },
        )
        .expect_err("zero-sum weights should be invalid");
    assert!(matches!(err, LoongMemoryError::Validation(_)));
}

#[test]
fn namespace_length_limit_is_enforced() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("memory.db");
    let audit = Arc::new(InMemoryAuditSink::default());
    let mut engine = build_engine(&db_path, allowed_policy_for("x"), audit);
    let ctx = loong_memory_core::OperationContext::new("validator");

    let long_namespace = "n".repeat(300);
    let err = engine
        .put(
            &ctx,
            &MemoryPutRequest {
                namespace: long_namespace,
                external_id: None,
                content: "payload".to_owned(),
                metadata: json!({}),
            },
        )
        .expect_err("long namespace should be invalid");
    assert!(matches!(err, LoongMemoryError::Validation(_)));
}

#[test]
fn recall_limit_is_respected_with_many_rows() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("memory.db");
    let audit = Arc::new(InMemoryAuditSink::default());
    let mut engine = build_engine(&db_path, allowed_policy_for("scale"), audit);
    let ctx = loong_memory_core::OperationContext::new("tester");

    for idx in 0..200 {
        engine
            .put(
                &ctx,
                &MemoryPutRequest {
                    namespace: "scale".to_owned(),
                    external_id: Some(format!("key-{idx}")),
                    content: format!("rust memory record index {idx}"),
                    metadata: json!({ "idx": idx }),
                },
            )
            .expect("bulk put");
    }

    let hits = engine
        .recall(
            &ctx,
            &RecallRequest {
                namespace: "scale".to_owned(),
                query: "rust memory".to_owned(),
                limit: 15,
                weights: ScoreWeights::default(),
            },
        )
        .expect("bulk recall");
    assert_eq!(hits.len(), 15);
    for window in hits.windows(2) {
        assert!(window[0].hybrid_score >= window[1].hybrid_score);
    }
}

#[test]
fn recall_rejects_excessive_limit() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("memory.db");
    let audit = Arc::new(InMemoryAuditSink::default());
    let mut engine = build_engine(&db_path, allowed_policy_for("scale"), audit);
    let ctx = loong_memory_core::OperationContext::new("tester");

    engine
        .put(
            &ctx,
            &MemoryPutRequest {
                namespace: "scale".to_owned(),
                external_id: Some("k1".to_owned()),
                content: "baseline".to_owned(),
                metadata: json!({}),
            },
        )
        .expect("seed");

    let err = engine
        .recall(
            &ctx,
            &RecallRequest {
                namespace: "scale".to_owned(),
                query: "baseline".to_owned(),
                limit: 10_000,
                weights: ScoreWeights::default(),
            },
        )
        .expect_err("too large limit should be rejected");
    assert!(matches!(err, LoongMemoryError::Validation(_)));
}

#[test]
fn multilingual_cjk_recall_returns_relevant_record() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("memory.db");
    let audit = Arc::new(InMemoryAuditSink::default());
    let mut engine = build_engine(&db_path, allowed_policy_for("multi"), audit);
    let ctx = loong_memory_core::OperationContext::new("tester");

    engine
        .put(
            &ctx,
            &MemoryPutRequest {
                namespace: "multi".to_owned(),
                external_id: Some("zh-1".to_owned()),
                content: "这个系统支持内存检索与上下文管理".to_owned(),
                metadata: json!({"lang":"zh"}),
            },
        )
        .expect("seed zh");
    engine
        .put(
            &ctx,
            &MemoryPutRequest {
                namespace: "multi".to_owned(),
                external_id: Some("en-1".to_owned()),
                content: "network timeout troubleshooting guide".to_owned(),
                metadata: json!({"lang":"en"}),
            },
        )
        .expect("seed en");

    let hits = engine
        .recall(
            &ctx,
            &RecallRequest {
                namespace: "multi".to_owned(),
                query: "内存检索".to_owned(),
                limit: 2,
                weights: ScoreWeights::default(),
            },
        )
        .expect("recall");

    assert_eq!(hits.len(), 2);
    assert!(hits[0].record.content.contains("内存检索"));
    assert!(hits[0].lexical_score > 0.0);
}

#[test]
fn vector_storage_uses_blob_and_reads_legacy_json_vectors() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("memory.db");
    let audit = Arc::new(InMemoryAuditSink::default());
    let mut engine = build_engine(&db_path, allowed_policy_for("vector"), audit);
    let ctx = loong_memory_core::OperationContext::new("tester");

    let created = engine
        .put(
            &ctx,
            &MemoryPutRequest {
                namespace: "vector".to_owned(),
                external_id: Some("v1".to_owned()),
                content: "vector baseline rust".to_owned(),
                metadata: json!({}),
            },
        )
        .expect("seed put");

    let conn = rusqlite::Connection::open(&db_path).expect("open raw sqlite");
    let kind: String = conn
        .query_row(
            "SELECT typeof(vector) FROM memory_vectors WHERE memory_id = ?1",
            rusqlite::params![created.id.as_str()],
            |row| row.get(0),
        )
        .expect("vector type");
    assert_eq!(kind, "blob");

    let embedder = DeterministicHashEmbedder::new(128);
    let vec = embedder.embed("vector baseline rust").expect("embed");
    let legacy_json = serde_json::to_string(&vec).expect("serialize legacy json");
    conn.execute(
        "UPDATE memory_vectors SET vector = ?1 WHERE memory_id = ?2",
        rusqlite::params![legacy_json, created.id.as_str()],
    )
    .expect("overwrite with legacy text vector");

    let hits = engine
        .recall(
            &ctx,
            &RecallRequest {
                namespace: "vector".to_owned(),
                query: "vector baseline rust".to_owned(),
                limit: 1,
                weights: ScoreWeights::default(),
            },
        )
        .expect("recall using legacy vector text");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].record.id, created.id);
}
