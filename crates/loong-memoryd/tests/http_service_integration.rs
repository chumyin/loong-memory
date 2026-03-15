use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use loong_memoryd::{serve_with_shutdown, ServiceConfig, ServiceState};
use reqwest::{Client, StatusCode};
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio::{net::TcpListener, sync::oneshot, task::JoinHandle};

struct TestServer {
    _temp_dir: TempDir,
    client: Client,
    base_url: String,
    shutdown_tx: Option<oneshot::Sender<()>>,
    join_handle: JoinHandle<()>,
}

impl TestServer {
    async fn spawn(policy_file: Option<&Path>) -> Result<Self> {
        let temp_dir = TempDir::new().context("create temp dir")?;
        let db_path = temp_dir.path().join("loong-memory.db");
        Self::spawn_with_security(temp_dir, db_path, policy_file, None).await
    }

    async fn spawn_with_db_path(
        temp_dir: TempDir,
        db_path: PathBuf,
        policy_file: Option<&Path>,
    ) -> Result<Self> {
        Self::spawn_with_security(temp_dir, db_path, policy_file, None).await
    }

    async fn spawn_with_security(
        temp_dir: TempDir,
        db_path: PathBuf,
        policy_file: Option<&Path>,
        _auth_file: Option<&Path>,
    ) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .context("bind tcp listener")?;
        let address = listener.local_addr().context("read listener address")?;
        let (shutdown_tx, shutdown_rx) = oneshot::channel();

        let config = ServiceConfig::new(db_path, policy_file.map(PathBuf::from));
        let state = ServiceState::from_config(&config)?;

        let join_handle = tokio::spawn(async move {
            let shutdown = async move {
                let _ = shutdown_rx.await;
            };
            if let Err(err) = serve_with_shutdown(listener, state, shutdown).await {
                panic!("server exited with error: {err}");
            }
        });

        Ok(Self {
            _temp_dir: temp_dir,
            client: Client::new(),
            base_url: format!("http://{address}"),
            shutdown_tx: Some(shutdown_tx),
            join_handle,
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }
        self.join_handle.abort();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn healthz_returns_ok_without_principal() -> Result<()> {
    let server = TestServer::spawn(None).await?;

    let response = server.client.get(server.url("/healthz")).send().await?;

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response.json().await?;
    assert_eq!(body["status"], "ok");
    assert_eq!(body["service"], "loong-memoryd");
    assert_eq!(body["policy_mode"], "allow_all");
    assert_eq!(body["auth_mode"], "trusted_header");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn put_requires_principal_header() -> Result<()> {
    let server = TestServer::spawn(None).await?;

    let response = server
        .client
        .post(server.url("/v1/memories"))
        .json(&json!({
            "namespace": "agent-demo",
            "external_id": "profile",
            "content": "Alice likes rust",
            "metadata": {"source": "test"}
        }))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body: Value = response.json().await?;
    assert_eq!(body["error"]["code"], "missing_principal");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn put_and_get_roundtrip_over_http() -> Result<()> {
    let server = TestServer::spawn(None).await?;

    let put_response = server
        .client
        .post(server.url("/v1/memories"))
        .header("x-loong-principal", "operator")
        .json(&json!({
            "namespace": "agent-demo",
            "external_id": "profile",
            "content": "Alice likes rust and sqlite",
            "metadata": {"source": "seed"}
        }))
        .send()
        .await?;

    assert_eq!(put_response.status(), StatusCode::OK);

    let get_response = server
        .client
        .post(server.url("/v1/memories/get"))
        .header("x-loong-principal", "operator")
        .json(&json!({
            "namespace": "agent-demo",
            "external_id": "profile"
        }))
        .send()
        .await?;

    assert_eq!(get_response.status(), StatusCode::OK);
    let body: Value = get_response.json().await?;
    assert_eq!(body["namespace"], "agent-demo");
    assert_eq!(body["external_id"], "profile");
    assert_eq!(body["content"], "Alice likes rust and sqlite");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recall_returns_relevant_hit() -> Result<()> {
    let server = TestServer::spawn(None).await?;

    let put_response = server
        .client
        .post(server.url("/v1/memories"))
        .header("x-loong-principal", "operator")
        .json(&json!({
            "namespace": "agent-demo",
            "external_id": "profile",
            "content": "Alice likes rust and sqlite",
            "metadata": {"source": "seed"}
        }))
        .send()
        .await?;
    assert_eq!(put_response.status(), StatusCode::OK);

    let recall_response = server
        .client
        .post(server.url("/v1/recall"))
        .header("x-loong-principal", "operator")
        .json(&json!({
            "namespace": "agent-demo",
            "query": "rust sqlite",
            "limit": 3
        }))
        .send()
        .await?;

    assert_eq!(recall_response.status(), StatusCode::OK);
    let body: Value = recall_response.json().await?;
    assert_eq!(body["count"], 1);
    assert_eq!(body["hits"][0]["record"]["external_id"], "profile");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn audit_returns_namespace_history_without_self_pollution() -> Result<()> {
    let server = TestServer::spawn(None).await?;

    let put_response = server
        .client
        .post(server.url("/v1/memories"))
        .header("x-loong-principal", "operator")
        .json(&json!({
            "namespace": "agent-demo",
            "external_id": "profile",
            "content": "Alice likes rust and sqlite",
            "metadata": {"source": "seed"}
        }))
        .send()
        .await?;
    assert_eq!(put_response.status(), StatusCode::OK);

    let audit_response = server
        .client
        .post(server.url("/v1/audit"))
        .header("x-loong-principal", "operator")
        .json(&json!({
            "namespace": "agent-demo",
            "limit": 20
        }))
        .send()
        .await?;

    assert_eq!(audit_response.status(), StatusCode::OK);
    let body: Value = audit_response.json().await?;
    assert!(body["count"].as_u64().unwrap_or(0) >= 2);
    let events = body["events"]
        .as_array()
        .context("events response should be an array")?;
    let actions: Vec<&str> = events
        .iter()
        .filter_map(|event| event["action"].as_str())
        .collect();
    assert!(actions.contains(&"put"));
    assert!(!actions.contains(&"audit_events"));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn static_policy_denial_returns_forbidden() -> Result<()> {
    let temp_dir = TempDir::new().context("create policy temp dir")?;
    let policy_path = temp_dir.path().join("policy.json");
    std::fs::write(&policy_path, "{}").context("write policy file")?;

    let server = TestServer::spawn(Some(&policy_path)).await?;

    let response = server
        .client
        .post(server.url("/v1/memories"))
        .header("x-loong-principal", "operator")
        .json(&json!({
            "namespace": "agent-demo",
            "external_id": "profile",
            "content": "Alice likes rust",
            "metadata": {"source": "test"}
        }))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let body: Value = response.json().await?;
    assert_eq!(body["error"]["code"], "policy_denied");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delete_removes_record_over_http() -> Result<()> {
    let server = TestServer::spawn(None).await?;

    let put_response = server
        .client
        .post(server.url("/v1/memories"))
        .header("x-loong-principal", "operator")
        .json(&json!({
            "namespace": "agent-demo",
            "external_id": "profile",
            "content": "Alice likes rust and sqlite",
            "metadata": {"source": "seed"}
        }))
        .send()
        .await?;
    assert_eq!(put_response.status(), StatusCode::OK);

    let delete_response = server
        .client
        .delete(server.url("/v1/memories"))
        .header("x-loong-principal", "operator")
        .json(&json!({
            "namespace": "agent-demo",
            "external_id": "profile"
        }))
        .send()
        .await?;

    assert_eq!(delete_response.status(), StatusCode::OK);
    let body: Value = delete_response.json().await?;
    assert_eq!(body["ok"], true);

    let get_response = server
        .client
        .post(server.url("/v1/memories/get"))
        .header("x-loong-principal", "operator")
        .json(&json!({
            "namespace": "agent-demo",
            "external_id": "profile"
        }))
        .send()
        .await?;
    assert_eq!(get_response.status(), StatusCode::NOT_FOUND);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delete_rejects_invalid_selector() -> Result<()> {
    let server = TestServer::spawn(None).await?;

    let delete_response = server
        .client
        .delete(server.url("/v1/memories"))
        .header("x-loong-principal", "operator")
        .json(&json!({
            "namespace": "agent-demo",
            "id": "memory-id",
            "external_id": "profile"
        }))
        .send()
        .await?;

    assert_eq!(delete_response.status(), StatusCode::BAD_REQUEST);
    let body: Value = delete_response.json().await?;
    assert_eq!(body["error"]["code"], "validation_failed");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn vector_health_returns_report_over_http() -> Result<()> {
    let server = TestServer::spawn(None).await?;

    let put_response = server
        .client
        .post(server.url("/v1/memories"))
        .header("x-loong-principal", "operator")
        .json(&json!({
            "namespace": "agent-demo",
            "external_id": "profile",
            "content": "Alice likes rust and sqlite",
            "metadata": {"source": "seed"}
        }))
        .send()
        .await?;
    assert_eq!(put_response.status(), StatusCode::OK);

    let response = server
        .client
        .post(server.url("/v1/vector-health"))
        .header("x-loong-principal", "operator")
        .json(&json!({
            "namespace": "agent-demo",
            "invalid_sample_limit": 5
        }))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response.json().await?;
    assert_eq!(body["namespace"], "agent-demo");
    assert_eq!(body["total_rows"], 1);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn vector_repair_returns_dry_run_report_over_http() -> Result<()> {
    let server = TestServer::spawn(None).await?;

    let response = server
        .client
        .post(server.url("/v1/vector-repair"))
        .header("x-loong-principal", "operator")
        .json(&json!({
            "namespace": "agent-demo",
            "issue_sample_limit": 5,
            "apply": false
        }))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response.json().await?;
    assert_eq!(body["namespace"], "agent-demo");
    assert_eq!(body["apply"], false);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn vector_health_policy_denial_returns_forbidden() -> Result<()> {
    let temp_dir = TempDir::new().context("create policy temp dir")?;
    let policy_path = temp_dir.path().join("policy.json");
    std::fs::write(&policy_path, "{}").context("write policy file")?;

    let server = TestServer::spawn(Some(&policy_path)).await?;

    let response = server
        .client
        .post(server.url("/v1/vector-health"))
        .header("x-loong-principal", "operator")
        .json(&json!({
            "namespace": "agent-demo",
            "invalid_sample_limit": 5
        }))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let body: Value = response.json().await?;
    assert_eq!(body["error"]["code"], "policy_denied");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn malformed_json_returns_invalid_json_error() -> Result<()> {
    let server = TestServer::spawn(None).await?;

    let response = server
        .client
        .post(server.url("/v1/memories"))
        .header("x-loong-principal", "operator")
        .header("content-type", "application/json")
        .body("{\"namespace\":")
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body: Value = response.json().await?;
    assert_eq!(body["error"]["code"], "invalid_json");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recall_rejects_invalid_weights_over_http() -> Result<()> {
    let server = TestServer::spawn(None).await?;

    let response = server
        .client
        .post(server.url("/v1/recall"))
        .header("x-loong-principal", "operator")
        .json(&json!({
            "namespace": "agent-demo",
            "query": "rust sqlite",
            "limit": 3,
            "lexical_weight": 0.0,
            "vector_weight": 0.0
        }))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body: Value = response.json().await?;
    assert_eq!(body["error"]["code"], "validation_failed");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn healthz_reports_readiness_failure_for_unopenable_db_path() -> Result<()> {
    let temp_dir = TempDir::new().context("create temp dir")?;
    let db_path = temp_dir.path().join("missing").join("loong-memory.db");
    let server = TestServer::spawn_with_db_path(temp_dir, db_path, None).await?;

    let response = server.client.get(server.url("/healthz")).send().await?;

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body: Value = response.json().await?;
    assert_eq!(body["error"]["code"], "internal_error");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bearer_token_allows_put_without_principal_header() -> Result<()> {
    let temp_dir = TempDir::new().context("create temp dir")?;
    let policy_path = temp_dir.path().join("policy.json");
    let auth_path = temp_dir.path().join("auth.json");

    std::fs::write(
        &policy_path,
        serde_json::to_string_pretty(&json!({
            "principal_namespace_actions": [
                {
                    "principal": "operator",
                    "namespace": "agent-demo",
                    "actions": ["put", "get", "recall", "delete", "audit_read", "repair"]
                }
            ]
        }))?,
    )
    .context("write policy file")?;
    std::fs::write(
        &auth_path,
        serde_json::to_string_pretty(&json!({
            "tokens": [
                {
                    "token": "operator-secret",
                    "principal": "operator"
                }
            ]
        }))?,
    )
    .context("write auth file")?;

    let db_path = temp_dir.path().join("loong-memory.db");
    let server = TestServer::spawn_with_security(temp_dir, db_path, Some(&policy_path), Some(&auth_path)).await?;

    let response = server
        .client
        .post(server.url("/v1/memories"))
        .header("authorization", "Bearer operator-secret")
        .json(&json!({
            "namespace": "agent-demo",
            "external_id": "profile",
            "content": "Alice likes rust",
            "metadata": {"source": "auth-test"}
        }))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn static_token_mode_rejects_missing_bearer_token() -> Result<()> {
    let temp_dir = TempDir::new().context("create temp dir")?;
    let auth_path = temp_dir.path().join("auth.json");
    std::fs::write(
        &auth_path,
        serde_json::to_string_pretty(&json!({
            "tokens": [
                {
                    "token": "operator-secret",
                    "principal": "operator"
                }
            ]
        }))?,
    )
    .context("write auth file")?;

    let db_path = temp_dir.path().join("loong-memory.db");
    let server = TestServer::spawn_with_security(temp_dir, db_path, None, Some(&auth_path)).await?;

    let response = server
        .client
        .post(server.url("/v1/memories"))
        .json(&json!({
            "namespace": "agent-demo",
            "external_id": "profile",
            "content": "Alice likes rust",
            "metadata": {"source": "auth-test"}
        }))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body: Value = response.json().await?;
    assert_eq!(body["error"]["code"], "missing_authentication");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn static_token_mode_rejects_invalid_bearer_token() -> Result<()> {
    let temp_dir = TempDir::new().context("create temp dir")?;
    let auth_path = temp_dir.path().join("auth.json");
    std::fs::write(
        &auth_path,
        serde_json::to_string_pretty(&json!({
            "tokens": [
                {
                    "token": "operator-secret",
                    "principal": "operator"
                }
            ]
        }))?,
    )
    .context("write auth file")?;

    let db_path = temp_dir.path().join("loong-memory.db");
    let server = TestServer::spawn_with_security(temp_dir, db_path, None, Some(&auth_path)).await?;

    let response = server
        .client
        .post(server.url("/v1/memories"))
        .header("authorization", "Bearer wrong-secret")
        .json(&json!({
            "namespace": "agent-demo",
            "external_id": "profile",
            "content": "Alice likes rust",
            "metadata": {"source": "auth-test"}
        }))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body: Value = response.json().await?;
    assert_eq!(body["error"]["code"], "invalid_authentication");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn static_token_mode_ignores_spoofed_principal_header() -> Result<()> {
    let temp_dir = TempDir::new().context("create temp dir")?;
    let policy_path = temp_dir.path().join("policy.json");
    let auth_path = temp_dir.path().join("auth.json");

    std::fs::write(
        &policy_path,
        serde_json::to_string_pretty(&json!({
            "principal_namespace_actions": [
                {
                    "principal": "operator",
                    "namespace": "agent-demo",
                    "actions": ["put", "get", "recall", "delete", "audit_read", "repair"]
                }
            ]
        }))?,
    )
    .context("write policy file")?;
    std::fs::write(
        &auth_path,
        serde_json::to_string_pretty(&json!({
            "tokens": [
                {
                    "token": "viewer-secret",
                    "principal": "viewer"
                }
            ]
        }))?,
    )
    .context("write auth file")?;

    let db_path = temp_dir.path().join("loong-memory.db");
    let server = TestServer::spawn_with_security(temp_dir, db_path, Some(&policy_path), Some(&auth_path)).await?;

    let response = server
        .client
        .post(server.url("/v1/memories"))
        .header("authorization", "Bearer viewer-secret")
        .header("x-loong-principal", "operator")
        .json(&json!({
            "namespace": "agent-demo",
            "external_id": "profile",
            "content": "Alice likes rust",
            "metadata": {"source": "auth-test"}
        }))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let body: Value = response.json().await?;
    assert_eq!(body["error"]["code"], "policy_denied");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn healthz_reports_static_token_auth_mode() -> Result<()> {
    let temp_dir = TempDir::new().context("create temp dir")?;
    let auth_path = temp_dir.path().join("auth.json");
    std::fs::write(
        &auth_path,
        serde_json::to_string_pretty(&json!({
            "tokens": [
                {
                    "token": "operator-secret",
                    "principal": "operator"
                }
            ]
        }))?,
    )
    .context("write auth file")?;

    let db_path = temp_dir.path().join("loong-memory.db");
    let server = TestServer::spawn_with_security(temp_dir, db_path, None, Some(&auth_path)).await?;

    let response = server.client.get(server.url("/healthz")).send().await?;

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response.json().await?;
    assert_eq!(body["auth_mode"], "static_token");
    Ok(())
}
