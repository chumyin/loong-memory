use std::{fs, path::Path, process::Command};

use serde_json::Value;
use tempfile::tempdir;

fn run_cli(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_loong-memory"))
        .args(args)
        .output()
        .expect("run loong-memory cli")
}

fn write_policy(path: &Path, body: &str) {
    fs::write(path, body).expect("write policy file");
}

fn parse_stdout_json(output: &std::process::Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("parse stdout json")
}

#[test]
fn audit_command_requires_principal() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("memory.db");
    let db = db_path.to_string_lossy().to_string();

    let init = run_cli(&["init", "--db", &db]);
    assert!(
        init.status.success(),
        "init stderr={}",
        String::from_utf8_lossy(&init.stderr)
    );

    let output = run_cli(&["audit", "--db", &db, "--namespace", "agent-demo"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--principal"));
}

#[test]
fn policy_file_allows_put_and_audit_for_operator() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("memory.db");
    let policy_path = dir.path().join("policy.json");
    let db = db_path.to_string_lossy().to_string();
    let policy = policy_path.to_string_lossy().to_string();

    write_policy(
        &policy_path,
        r#"{
  "principal_namespace_actions": [
    {
      "principal": "operator",
      "namespace": "agent-demo",
      "actions": ["put", "audit_read"]
    }
  ]
}"#,
    );

    let init = run_cli(&["init", "--db", &db]);
    assert!(
        init.status.success(),
        "init stderr={}",
        String::from_utf8_lossy(&init.stderr)
    );

    let put = run_cli(&[
        "--policy-file",
        &policy,
        "put",
        "--db",
        &db,
        "--namespace",
        "agent-demo",
        "--external-id",
        "profile",
        "--content",
        "operator seeded memory",
        "--principal",
        "operator",
    ]);
    assert!(
        put.status.success(),
        "put stderr={}",
        String::from_utf8_lossy(&put.stderr)
    );

    let audit = run_cli(&[
        "--policy-file",
        &policy,
        "audit",
        "--db",
        &db,
        "--namespace",
        "agent-demo",
        "--limit",
        "20",
        "--principal",
        "operator",
    ]);
    assert!(
        audit.status.success(),
        "audit stderr={}",
        String::from_utf8_lossy(&audit.stderr)
    );

    let payload = parse_stdout_json(&audit);
    let events = payload["events"].as_array().expect("events array");
    assert!(events.iter().any(|evt| evt["action"] == "Put"));
    assert!(events.iter().any(|evt| evt["action"] == "put"));
    assert!(!events.iter().any(|evt| evt["action"] == "AuditRead"));
    assert!(!events.iter().any(|evt| evt["action"] == "audit_events"));
}

#[test]
fn policy_file_denies_audit_without_audit_read_permission() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("memory.db");
    let policy_path = dir.path().join("policy.json");
    let db = db_path.to_string_lossy().to_string();
    let policy = policy_path.to_string_lossy().to_string();

    write_policy(
        &policy_path,
        r#"{
  "principal_namespace_actions": [
    {
      "principal": "operator",
      "namespace": "agent-demo",
      "actions": ["put"]
    }
  ]
}"#,
    );

    let init = run_cli(&["init", "--db", &db]);
    assert!(
        init.status.success(),
        "init stderr={}",
        String::from_utf8_lossy(&init.stderr)
    );

    let put = run_cli(&[
        "--policy-file",
        &policy,
        "put",
        "--db",
        &db,
        "--namespace",
        "agent-demo",
        "--external-id",
        "profile",
        "--content",
        "operator seeded memory",
        "--principal",
        "operator",
    ]);
    assert!(
        put.status.success(),
        "put stderr={}",
        String::from_utf8_lossy(&put.stderr)
    );

    let audit = run_cli(&[
        "--policy-file",
        &policy,
        "audit",
        "--db",
        &db,
        "--namespace",
        "agent-demo",
        "--limit",
        "20",
        "--principal",
        "operator",
    ]);
    assert!(!audit.status.success());
    assert!(String::from_utf8_lossy(&audit.stderr).contains("policy denied"));
}
