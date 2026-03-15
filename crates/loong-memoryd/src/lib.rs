#![forbid(unsafe_code)]

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::{
    extract::{rejection::JsonRejection, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use loong_memory_core::{
    AllowAllPolicy, DeterministicHashEmbedder, EmbeddingProvider, EngineConfig, LoongMemoryError,
    MemoryEngine, MemoryGetRequest, MemoryPutRequest, OperationContext, PolicyEngine,
    RecallRequest, ScoreWeights, SqliteAuditSink, SqliteStore, StaticPolicy, StaticPolicyConfig,
};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tracing::error;

pub const PRINCIPAL_HEADER: &str = "x-loong-principal";

#[derive(Debug, Clone)]
pub struct ServiceConfig {
    db_path: PathBuf,
    policy_file: Option<PathBuf>,
}

impl ServiceConfig {
    pub fn new(db_path: PathBuf, policy_file: Option<PathBuf>) -> Self {
        Self {
            db_path,
            policy_file,
        }
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    pub fn policy_file(&self) -> Option<&Path> {
        self.policy_file.as_deref()
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyMode {
    AllowAll,
    Static,
}

#[derive(Clone)]
pub struct ServiceState {
    db_path: PathBuf,
    policy: Arc<dyn PolicyEngine>,
    embedder: Arc<dyn EmbeddingProvider>,
    policy_mode: PolicyMode,
}

impl ServiceState {
    pub fn from_config(config: &ServiceConfig) -> Result<Self> {
        let (policy, policy_mode) = load_policy(config.policy_file())?;
        Ok(Self {
            db_path: config.db_path().to_path_buf(),
            policy,
            embedder: Arc::new(DeterministicHashEmbedder::default()),
            policy_mode,
        })
    }
}

#[derive(Debug, Deserialize)]
struct RecallRequestBody {
    namespace: String,
    query: String,
    #[serde(default = "default_recall_limit")]
    limit: usize,
    #[serde(default = "default_lexical_weight")]
    lexical_weight: f32,
    #[serde(default = "default_vector_weight")]
    vector_weight: f32,
}

#[derive(Debug, Deserialize)]
struct AuditRequestBody {
    namespace: String,
    #[serde(default = "default_audit_limit")]
    limit: usize,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    service: &'static str,
    db: String,
    policy_mode: PolicyMode,
}

#[derive(Debug, Serialize)]
struct CountEnvelope<T> {
    count: usize,
    #[serde(flatten)]
    inner: T,
}

#[derive(Debug, Serialize)]
struct HitsBody {
    hits: Vec<loong_memory_core::RecallHit>,
}

#[derive(Debug, Serialize)]
struct AuditEventsBody {
    events: Vec<loong_memory_core::AuditEvent>,
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

#[derive(Debug, Serialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    code: &'static str,
    message: String,
}

impl ApiError {
    fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }

    fn validation(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "validation_failed", message)
    }

    fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "missing_principal", message)
    }

    fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorEnvelope {
                error: ErrorBody {
                    code: self.code,
                    message: self.message,
                },
            }),
        )
            .into_response()
    }
}

impl From<LoongMemoryError> for ApiError {
    fn from(err: LoongMemoryError) -> Self {
        match err {
            LoongMemoryError::Validation(message) => ApiError::validation(message),
            LoongMemoryError::PolicyDenied(message) => {
                ApiError::new(StatusCode::FORBIDDEN, "policy_denied", message)
            }
            LoongMemoryError::NotFound => {
                ApiError::new(StatusCode::NOT_FOUND, "not_found", "not found")
            }
            LoongMemoryError::Storage(message) | LoongMemoryError::Internal(message) => {
                error!(error = %message, "service request failed with internal/storage error");
                ApiError::internal("internal error")
            }
            LoongMemoryError::NotImplemented(message) => {
                error!(error = %message, "service request failed with internal/storage error");
                ApiError::internal("internal error")
            }
        }
    }
}

impl From<JsonRejection> for ApiError {
    fn from(rejection: JsonRejection) -> Self {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid_json",
            rejection.body_text(),
        )
    }
}

pub fn app(state: ServiceState) -> Router {
    Router::new()
        .route("/healthz", get(health))
        .route("/v1/memories", post(put_memory))
        .route("/v1/memories/get", post(get_memory))
        .route("/v1/recall", post(recall))
        .route("/v1/audit", post(audit))
        .with_state(state)
}

pub async fn serve_with_shutdown<F>(
    listener: TcpListener,
    state: ServiceState,
    shutdown: F,
) -> Result<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    axum::serve(listener, app(state))
        .with_graceful_shutdown(shutdown)
        .await
        .context("serve loong-memoryd")?;
    Ok(())
}

async fn health(State(state): State<ServiceState>) -> Result<Json<HealthResponse>, ApiError> {
    let db_path = state.db_path.clone();
    tokio::task::spawn_blocking(move || -> Result<(), LoongMemoryError> {
        let _store = SqliteStore::open(&db_path)?;
        let _audit = SqliteAuditSink::open(&db_path)?;
        Ok(())
    })
    .await
    .map_err(|err| {
        error!(error = %err, "health worker panicked");
        ApiError::internal("internal error")
    })?
    .map_err(ApiError::from)?;

    Ok(Json(HealthResponse {
        status: "ok",
        service: "loong-memoryd",
        db: state.db_path.display().to_string(),
        policy_mode: state.policy_mode,
    }))
}

async fn put_memory(
    State(state): State<ServiceState>,
    headers: HeaderMap,
    body: Result<Json<MemoryPutRequest>, JsonRejection>,
) -> Result<Json<loong_memory_core::MemoryRecord>, ApiError> {
    let principal = extract_principal(&headers)?;
    let Json(req) = body.map_err(ApiError::from)?;
    let record = with_engine(state, principal, move |engine, ctx| engine.put(&ctx, &req)).await?;
    Ok(Json(record))
}

async fn get_memory(
    State(state): State<ServiceState>,
    headers: HeaderMap,
    body: Result<Json<MemoryGetRequest>, JsonRejection>,
) -> Result<Json<loong_memory_core::MemoryRecord>, ApiError> {
    let principal = extract_principal(&headers)?;
    let Json(req) = body.map_err(ApiError::from)?;
    let record = with_engine(state, principal, move |engine, ctx| engine.get(&ctx, &req)).await?;
    Ok(Json(record))
}

async fn recall(
    State(state): State<ServiceState>,
    headers: HeaderMap,
    body: Result<Json<RecallRequestBody>, JsonRejection>,
) -> Result<Json<CountEnvelope<HitsBody>>, ApiError> {
    let principal = extract_principal(&headers)?;
    let Json(req) = body.map_err(ApiError::from)?;
    let weights = normalize_weights(req.lexical_weight, req.vector_weight)?;
    let request = RecallRequest {
        namespace: req.namespace,
        query: req.query,
        limit: req.limit,
        weights,
    };
    let hits = with_engine(state, principal, move |engine, ctx| {
        engine.recall(&ctx, &request)
    })
    .await?;
    Ok(Json(CountEnvelope {
        count: hits.len(),
        inner: HitsBody { hits },
    }))
}

async fn audit(
    State(state): State<ServiceState>,
    headers: HeaderMap,
    body: Result<Json<AuditRequestBody>, JsonRejection>,
) -> Result<Json<CountEnvelope<AuditEventsBody>>, ApiError> {
    let principal = extract_principal(&headers)?;
    let Json(req) = body.map_err(ApiError::from)?;
    let events = with_engine(state, principal, move |engine, ctx| {
        engine.audit_events(&ctx, &req.namespace, req.limit)
    })
    .await?;
    Ok(Json(CountEnvelope {
        count: events.len(),
        inner: AuditEventsBody { events },
    }))
}

fn default_recall_limit() -> usize {
    5
}

fn default_lexical_weight() -> f32 {
    0.55
}

fn default_vector_weight() -> f32 {
    0.45
}

fn default_audit_limit() -> usize {
    50
}

fn extract_principal(headers: &HeaderMap) -> Result<String, ApiError> {
    let Some(value) = headers.get(PRINCIPAL_HEADER) else {
        return Err(ApiError::unauthorized(format!(
            "missing required header {PRINCIPAL_HEADER}"
        )));
    };
    let principal = value
        .to_str()
        .map_err(|_| ApiError::unauthorized(format!("invalid header {PRINCIPAL_HEADER}")))?
        .trim()
        .to_owned();
    if principal.is_empty() {
        return Err(ApiError::unauthorized(format!(
            "missing required header {PRINCIPAL_HEADER}"
        )));
    }
    Ok(principal)
}

fn normalize_weights(lexical: f32, vector: f32) -> Result<ScoreWeights, ApiError> {
    if !lexical.is_finite() || !vector.is_finite() {
        return Err(ApiError::validation("weights must be finite numbers"));
    }
    let sum = lexical + vector;
    if sum <= 0.0 {
        return Err(ApiError::validation(
            "lexical/vector weights must sum to a positive value",
        ));
    }
    Ok(ScoreWeights {
        lexical: lexical / sum,
        vector: vector / sum,
    })
}

async fn with_engine<R, F>(state: ServiceState, principal: String, op: F) -> Result<R, ApiError>
where
    R: Send + 'static,
    F: FnOnce(&mut MemoryEngine<SqliteStore>, OperationContext) -> Result<R, LoongMemoryError>
        + Send
        + 'static,
{
    let db_path = state.db_path.clone();
    let policy = Arc::clone(&state.policy);
    let embedder = Arc::clone(&state.embedder);

    let result = tokio::task::spawn_blocking(move || -> Result<R, LoongMemoryError> {
        let store = SqliteStore::open(&db_path)?;
        let audit = Arc::new(SqliteAuditSink::open(&db_path)?);
        let mut engine = MemoryEngine::new(store, policy, embedder, audit, EngineConfig::default());
        op(&mut engine, OperationContext::new(principal))
    })
    .await
    .map_err(|err| {
        error!(error = %err, "request worker panicked");
        ApiError::internal("internal error")
    })?;

    result.map_err(ApiError::from)
}

fn load_policy(policy_file: Option<&Path>) -> Result<(Arc<dyn PolicyEngine>, PolicyMode)> {
    match policy_file {
        Some(path) => {
            let raw = std::fs::read_to_string(path)
                .with_context(|| format!("read policy file {}", path.display()))?;
            let config: StaticPolicyConfig = serde_json::from_str(&raw)
                .with_context(|| format!("parse policy file {}", path.display()))?;
            Ok((
                Arc::new(StaticPolicy::from_config(config)),
                PolicyMode::Static,
            ))
        }
        None => Ok((Arc::new(AllowAllPolicy), PolicyMode::AllowAll)),
    }
}
