use std::path::PathBuf;

use anyhow::{Context, Result};
use loong_memoryd::{ServiceConfig, ServiceState};
use serde_json::json;
use tempfile::TempDir;

#[test]
fn service_state_rejects_duplicate_auth_tokens() -> Result<()> {
    let temp_dir = TempDir::new().context("create temp dir")?;
    let auth_path = temp_dir.path().join("auth.json");
    std::fs::write(
        &auth_path,
        serde_json::to_string_pretty(&json!({
            "tokens": [
                {
                    "token": "shared-secret",
                    "principal": "operator"
                },
                {
                    "token": "shared-secret",
                    "principal": "viewer"
                }
            ]
        }))?,
    )
    .context("write auth file")?;

    let config = ServiceConfig::new(
        temp_dir.path().join("loong-memory.db"),
        None,
        Some(PathBuf::from(&auth_path)),
    );
    let err = match ServiceState::from_config(&config) {
        Ok(_) => panic!("duplicate token should fail"),
        Err(err) => err,
    };

    assert!(err.to_string().contains("duplicate token entries"));
    Ok(())
}

#[test]
fn service_state_rejects_empty_auth_principal() -> Result<()> {
    let temp_dir = TempDir::new().context("create temp dir")?;
    let auth_path = temp_dir.path().join("auth.json");
    std::fs::write(
        &auth_path,
        serde_json::to_string_pretty(&json!({
            "tokens": [
                {
                    "token": "operator-secret",
                    "principal": "   "
                }
            ]
        }))?,
    )
    .context("write auth file")?;

    let config = ServiceConfig::new(
        temp_dir.path().join("loong-memory.db"),
        None,
        Some(PathBuf::from(&auth_path)),
    );
    let err = match ServiceState::from_config(&config) {
        Ok(_) => panic!("empty principal should fail"),
        Err(err) => err,
    };

    assert!(err.to_string().contains("empty principal"));
    Ok(())
}
