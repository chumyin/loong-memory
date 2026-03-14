use chrono::{Duration, Utc};
use loong_memory_core::{
    AuditEvent, AuditEventKind, AuditSink, LoongMemoryError, SqliteAuditLog, SqliteAuditSink,
};
use serde_json::json;
use tempfile::tempdir;

#[test]
fn sqlite_audit_sink_persists_and_log_reads_back() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("audit.db");

    let sink = SqliteAuditSink::open(&db_path).expect("open sink");
    let t0 = Utc::now();

    for idx in 0..3 {
        sink.record(AuditEvent {
            event_id: format!("evt-{idx}"),
            timestamp: t0 + Duration::milliseconds(idx as i64),
            principal: "tester".to_owned(),
            namespace: if idx % 2 == 0 {
                "alpha".to_owned()
            } else {
                "beta".to_owned()
            },
            action: "put".to_owned(),
            kind: AuditEventKind::Write,
            detail: json!({ "idx": idx }),
        })
        .expect("record event");
    }

    let log = SqliteAuditLog::open(&db_path).expect("open log");
    let all = log.list(None, 10).expect("list all");
    assert_eq!(all.len(), 3);
    assert_eq!(all[0].event_id, "evt-2");
    assert_eq!(all[1].event_id, "evt-1");
    assert_eq!(all[2].event_id, "evt-0");

    let alpha_only = log.list(Some("alpha"), 10).expect("list alpha");
    assert_eq!(alpha_only.len(), 2);
    assert!(alpha_only.iter().all(|evt| evt.namespace == "alpha"));

    let found = log
        .get_by_id("evt-1")
        .expect("get by id")
        .expect("event exists");
    assert_eq!(found.namespace, "beta");
    assert_eq!(found.kind.as_str(), "write");

    let missing = log.get_by_id("evt-missing").expect("query missing");
    assert!(missing.is_none());
}

#[test]
fn sqlite_audit_sink_rejects_duplicate_event_ids() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("audit.db");

    let sink = SqliteAuditSink::open(&db_path).expect("open sink");
    let event = AuditEvent {
        event_id: "dup-1".to_owned(),
        timestamp: Utc::now(),
        principal: "tester".to_owned(),
        namespace: "alpha".to_owned(),
        action: "put".to_owned(),
        kind: AuditEventKind::Write,
        detail: json!({ "rev": 1 }),
    };

    sink.record(event.clone()).expect("insert original event");
    let err = sink
        .record(AuditEvent {
            detail: json!({ "rev": 2 }),
            ..event
        })
        .expect_err("duplicate event ids should fail");
    assert!(matches!(err, LoongMemoryError::Storage(_)));

    let log = SqliteAuditLog::open(&db_path).expect("open log");
    let stored = log
        .get_by_id("dup-1")
        .expect("get by id")
        .expect("event exists");
    assert_eq!(stored.detail, json!({ "rev": 1 }));
}

#[test]
fn sqlite_audit_log_limit_is_clamped() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("audit.db");

    let sink = SqliteAuditSink::open(&db_path).expect("open sink");
    for idx in 0..5 {
        sink.record(AuditEvent {
            event_id: format!("limit-{idx}"),
            timestamp: Utc::now() + Duration::milliseconds(idx as i64),
            principal: "tester".to_owned(),
            namespace: "ns".to_owned(),
            action: "op".to_owned(),
            kind: AuditEventKind::Read,
            detail: json!({}),
        })
        .expect("record event");
    }

    let log = SqliteAuditLog::open(&db_path).expect("open log");
    let one = log.list(None, 0).expect("clamped to at least 1");
    assert_eq!(one.len(), 1);

    let many = log.list(None, 9_999).expect("clamped upper bound");
    assert_eq!(many.len(), 5);
}

#[test]
fn sqlite_audit_log_surfaces_parse_errors() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("audit.db");
    let _sink = SqliteAuditSink::open(&db_path).expect("open sink and init schema");

    let conn = rusqlite::Connection::open(&db_path).expect("open raw sqlite");
    conn.execute(
        r#"
        INSERT INTO memory_audit(event_id, timestamp, principal, namespace, action, kind, detail)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        "#,
        rusqlite::params!["bad-1", "not-a-time", "p", "ns", "x", "write", "{\"a\":1}"],
    )
    .expect("insert malformed timestamp");

    let log = SqliteAuditLog::open(&db_path).expect("open log");
    let err = log
        .list(None, 10)
        .expect_err("should fail on malformed timestamp");
    assert!(matches!(err, LoongMemoryError::Storage(_)));
}
