use std::sync::{Arc, Mutex};

use loong_memory_core::{
    Action, AllowAllPolicy, AuditEventKind, AuditSink, DeterministicHashEmbedder,
    EmbeddingProvider, EngineConfig, InMemoryAuditSink, LoongMemoryError, MemoryDeleteRequest,
    MemoryEngine, MemoryGetRequest, MemoryPutRequest, RecallRequest, ScoreWeights, SqliteAuditLog,
    SqliteAuditSink, SqliteStore, StaticPolicy,
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
            Action::Repair,
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

fn build_sqlite_audit_engine(
    db_path: &std::path::Path,
    policy: Arc<dyn loong_memory_core::PolicyEngine>,
) -> MemoryEngine<SqliteStore> {
    let store = SqliteStore::open(db_path).expect("open sqlite");
    let embedder = Arc::new(DeterministicHashEmbedder::new(128));
    let audit_sink: Arc<dyn AuditSink> =
        Arc::new(SqliteAuditSink::open(db_path).expect("open sqlite audit sink"));
    MemoryEngine::new(store, policy, embedder, audit_sink, EngineConfig::default())
}

#[derive(Debug)]
struct FailingAuditSink {
    fail_on_call: usize,
    calls: Mutex<usize>,
}

impl FailingAuditSink {
    fn new(fail_on_call: usize) -> Self {
        Self {
            fail_on_call,
            calls: Mutex::new(0),
        }
    }
}

impl AuditSink for FailingAuditSink {
    fn record(&self, _event: loong_memory_core::AuditEvent) -> Result<(), LoongMemoryError> {
        let mut calls = self.calls.lock().map_err(|_| {
            LoongMemoryError::Internal("failing audit sink lock poisoned".to_owned())
        })?;
        *calls += 1;
        if *calls == self.fail_on_call {
            return Err(LoongMemoryError::Storage(
                "forced audit sink failure".to_owned(),
            ));
        }
        Ok(())
    }
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
fn put_surfaces_post_write_audit_failure() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("memory.db");
    let policy = Arc::new(AllowAllPolicy);
    let audit_sink: Arc<dyn AuditSink> = Arc::new(FailingAuditSink::new(2));
    let store = SqliteStore::open(&db_path).expect("open sqlite");
    let embedder = Arc::new(DeterministicHashEmbedder::new(128));
    let mut engine =
        MemoryEngine::new(store, policy, embedder, audit_sink, EngineConfig::default());
    let ctx = loong_memory_core::OperationContext::new("tester");

    let err = engine
        .put(
            &ctx,
            &MemoryPutRequest {
                namespace: "agent-a".to_owned(),
                external_id: Some("profile".to_owned()),
                content: "Alice likes rust systems programming".to_owned(),
                metadata: json!({"source":"seed"}),
            },
        )
        .expect_err("audit failure should be surfaced");
    assert!(matches!(err, LoongMemoryError::Storage(_)));

    let verify = build_engine(
        &db_path,
        Arc::new(AllowAllPolicy),
        Arc::new(InMemoryAuditSink::default()),
    );
    let stored = verify
        .get(
            &ctx,
            &MemoryGetRequest {
                namespace: "agent-a".to_owned(),
                id: None,
                external_id: Some("profile".to_owned()),
            },
        )
        .expect("write committed before audit failure surfaced");
    assert_eq!(stored.content, "Alice likes rust systems programming");
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

#[test]
fn vector_health_and_repair_are_policy_gated() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("memory.db");
    let audit = Arc::new(InMemoryAuditSink::default());
    let policy = Arc::new(
        StaticPolicy::default().allow_namespace_actions("ops", [Action::AuditRead, Action::Put]),
    );
    let mut engine = build_engine(&db_path, policy, Arc::clone(&audit));
    let ctx = loong_memory_core::OperationContext::new("ops-user");

    engine
        .put(
            &ctx,
            &MemoryPutRequest {
                namespace: "ops".to_owned(),
                external_id: Some("k1".to_owned()),
                content: "ops row".to_owned(),
                metadata: json!({}),
            },
        )
        .expect("seed put");

    let report = engine
        .vector_health(&ctx, "ops", 5)
        .expect("vector health should be allowed");
    assert_eq!(report.total_rows, 1);

    let err = engine
        .vector_repair(&ctx, "ops", 5, false)
        .expect_err("vector repair should be denied without Action::Repair");
    assert!(matches!(err, LoongMemoryError::PolicyDenied(_)));

    let events = audit.snapshot();
    assert!(events
        .iter()
        .any(|evt| evt.action == "vector_health" && matches!(evt.kind, AuditEventKind::Read)));
    assert!(events
        .iter()
        .any(|evt| evt.action == "Repair" && matches!(evt.kind, AuditEventKind::Denied)));
}

#[test]
fn recall_skips_corrupted_vector_blob_instead_of_failing() {
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
                external_id: Some("bad-blob".to_owned()),
                content: "resilient vector recall path".to_owned(),
                metadata: json!({}),
            },
        )
        .expect("seed put");

    let conn = rusqlite::Connection::open(&db_path).expect("open raw sqlite");
    conn.execute(
        "UPDATE memory_vectors SET vector = ?1 WHERE memory_id = ?2",
        rusqlite::params![vec![0_u8, 1_u8, 2_u8], created.id.as_str()],
    )
    .expect("overwrite with malformed blob");

    let hits = engine
        .recall(
            &ctx,
            &RecallRequest {
                namespace: "vector".to_owned(),
                query: "resilient vector recall path".to_owned(),
                limit: 1,
                weights: ScoreWeights::default(),
            },
        )
        .expect("recall should not fail on malformed vector blob");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].record.id, created.id);
    assert_eq!(hits[0].vector_score, 0.0);
    assert!(hits[0].lexical_score > 0.0);
}

#[test]
fn recall_skips_non_finite_vector_values() {
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
                external_id: Some("nan-vec".to_owned()),
                content: "non finite vector resilience".to_owned(),
                metadata: json!({}),
            },
        )
        .expect("seed put");

    let conn = rusqlite::Connection::open(&db_path).expect("open raw sqlite");
    let nan_blob = vec![f32::NAN; 128]
        .into_iter()
        .flat_map(|v| v.to_le_bytes())
        .collect::<Vec<u8>>();
    conn.execute(
        "UPDATE memory_vectors SET vector = ?1 WHERE memory_id = ?2",
        rusqlite::params![nan_blob, created.id.as_str()],
    )
    .expect("overwrite with nan vector blob");

    let hits = engine
        .recall(
            &ctx,
            &RecallRequest {
                namespace: "vector".to_owned(),
                query: "non finite vector resilience".to_owned(),
                limit: 1,
                weights: ScoreWeights::default(),
            },
        )
        .expect("recall should not fail on nan vector blob");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].record.id, created.id);
    assert_eq!(hits[0].vector_score, 0.0);
    assert!(hits[0].lexical_score > 0.0);
}

#[test]
fn audit_read_without_reader_is_not_implemented() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("memory.db");
    let audit = Arc::new(InMemoryAuditSink::default());
    let mut engine = build_engine(&db_path, allowed_policy_for("ops"), audit);
    let ctx = loong_memory_core::OperationContext::new("ops-user");

    engine
        .put(
            &ctx,
            &MemoryPutRequest {
                namespace: "ops".to_owned(),
                external_id: Some("k1".to_owned()),
                content: "seed audit row".to_owned(),
                metadata: json!({}),
            },
        )
        .expect("seed put");

    let err = engine
        .audit_events(&ctx, "ops", 10)
        .expect_err("audit events require a configured reader");
    assert!(matches!(err, LoongMemoryError::NotImplemented(_)));
}

#[test]
fn audit_read_is_policy_gated_and_emits_denied_event() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("memory.db");
    let policy = Arc::new(StaticPolicy::default().allow_namespace_actions("ops", [Action::Put]));
    let mut engine = build_sqlite_audit_engine(&db_path, policy);
    let ctx = loong_memory_core::OperationContext::new("ops-user");

    engine
        .put(
            &ctx,
            &MemoryPutRequest {
                namespace: "ops".to_owned(),
                external_id: Some("k1".to_owned()),
                content: "seed audit row".to_owned(),
                metadata: json!({}),
            },
        )
        .expect("seed put");

    let err = engine
        .audit_events(&ctx, "ops", 10)
        .expect_err("audit read should be denied");
    assert!(matches!(err, LoongMemoryError::PolicyDenied(_)));

    let log = SqliteAuditLog::open(&db_path).expect("open sqlite audit log");
    let events = log.list(Some("ops"), 20).expect("list audit log");
    assert!(events
        .iter()
        .any(|evt| evt.action == "AuditRead" && matches!(evt.kind, AuditEventKind::Denied)));
}

#[test]
fn audit_read_excludes_self_generated_audit_events_from_results() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("memory.db");
    let policy = Arc::new(
        StaticPolicy::default().allow_namespace_actions("ops", [Action::Put, Action::AuditRead]),
    );
    let mut engine = build_sqlite_audit_engine(&db_path, policy);
    let ctx = loong_memory_core::OperationContext::new("ops-user");

    engine
        .put(
            &ctx,
            &MemoryPutRequest {
                namespace: "ops".to_owned(),
                external_id: Some("k1".to_owned()),
                content: "seed audit row".to_owned(),
                metadata: json!({}),
            },
        )
        .expect("seed put");

    let events = engine
        .audit_events(&ctx, "ops", 20)
        .expect("audit read should succeed");
    assert!(!events.iter().any(|evt| evt.action == "AuditRead"));
    assert!(!events.iter().any(|evt| evt.action == "audit_events"));
    assert!(events.iter().any(|evt| evt.action == "Put"));
    assert!(events.iter().any(|evt| evt.action == "put"));

    let log = SqliteAuditLog::open(&db_path).expect("open sqlite audit log");
    let persisted = log.list(Some("ops"), 20).expect("list audit log");
    assert!(persisted
        .iter()
        .any(|evt| evt.action == "AuditRead" && matches!(evt.kind, AuditEventKind::Allowed)));
    assert!(persisted
        .iter()
        .any(|evt| evt.action == "audit_events" && matches!(evt.kind, AuditEventKind::Read)));
}
