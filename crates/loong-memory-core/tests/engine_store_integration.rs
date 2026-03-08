use std::sync::Arc;

use loong_memory_core::{
    Action, AllowAllPolicy, AuditEventKind, AuditSink, DeterministicHashEmbedder, EngineConfig,
    InMemoryAuditSink, LoongMemoryError, MemoryDeleteRequest, MemoryEngine, MemoryGetRequest,
    MemoryPutRequest, RecallRequest, ScoreWeights, SqliteStore, StaticPolicy,
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
