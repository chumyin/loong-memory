#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use axum::{
    extract::{rejection::JsonRejection, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use loong_memory_core::{
    AllowAllPolicy, DeterministicHashEmbedder, EmbeddingProvider, EngineConfig, LoongMemoryError,
    MemoryDeleteRequest, MemoryEngine, MemoryGetRequest, MemoryPutRequest, OperationContext,
    PolicyEngine, RecallRequest, ScoreWeights, SqliteAuditSink, SqliteStore, StaticPolicy,
    StaticPolicyConfig,
};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tracing::error;

pub const PRINCIPAL_HEADER: &str = "x-loong-principal";

#[derive(Debug, Clone)]
pub struct ServiceConfig {
    db_path: PathBuf,
    policy_file: Option<PathBuf>,
    auth_file: Option<PathBuf>,
}

impl ServiceConfig {
    pub fn new(db_path: PathBuf, policy_file: Option<PathBuf>, auth_file: Option<PathBuf>) -> Self {
        Self {
            db_path,
            policy_file,
            auth_file,
        }
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    pub fn policy_file(&self) -> Option<&Path> {
        self.policy_file.as_deref()
    }

    pub fn auth_file(&self) -> Option<&Path> {
        self.auth_file.as_deref()
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyMode {
    AllowAll,
    Static,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthMode {
    TrustedHeader,
    StaticToken,
}

#[derive(Debug, Deserialize)]
struct StaticAuthConfig {
    #[serde(default)]
    tokens: Vec<StaticTokenConfig>,
}

#[derive(Debug, Deserialize)]
struct StaticTokenConfig {
    token: String,
    principal: String,
}

#[derive(Clone)]
enum Authenticator {
    TrustedHeader,
    StaticToken {
        token_to_principal: Arc<HashMap<String, String>>,
    },
}

impl Authenticator {
    fn mode(&self) -> AuthMode {
        match self {
            Self::TrustedHeader => AuthMode::TrustedHeader,
            Self::StaticToken { .. } => AuthMode::StaticToken,
        }
    }

    fn authenticate(&self, headers: &HeaderMap) -> Result<String, ApiError> {
        match self {
            Self::TrustedHeader => extract_trusted_header_principal(headers),
            Self::StaticToken { token_to_principal } => {
                let token = extract_bearer_token(headers)?;
                token_to_principal
                    .get(token)
                    .cloned()
                    .ok_or_else(|| ApiError::invalid_authentication("invalid bearer token"))
            }
        }
    }
}

#[derive(Clone)]
pub struct ServiceState {
    db_path: PathBuf,
    policy: Arc<dyn PolicyEngine>,
    embedder: Arc<dyn EmbeddingProvider>,
    policy_mode: PolicyMode,
    authenticator: Authenticator,
}

impl ServiceState {
    pub fn from_config(config: &ServiceConfig) -> Result<Self> {
        let (policy, policy_mode) = load_policy(config.policy_file())?;
        let authenticator = load_authenticator(config.auth_file())?;
        Ok(Self {
            db_path: config.db_path().to_path_buf(),
            policy,
            embedder: Arc::new(DeterministicHashEmbedder::default()),
            policy_mode,
            authenticator,
        })
    }

    pub fn auth_mode(&self) -> AuthMode {
        self.authenticator.mode()
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

#[derive(Debug, Deserialize)]
struct VectorHealthRequestBody {
    namespace: String,
    #[serde(default = "default_invalid_sample_limit")]
    invalid_sample_limit: usize,
}

#[derive(Debug, Deserialize)]
struct VectorRepairRequestBody {
    namespace: String,
    #[serde(default = "default_issue_sample_limit")]
    issue_sample_limit: usize,
    #[serde(default)]
    apply: bool,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    service: &'static str,
    db: String,
    policy_mode: PolicyMode,
    auth_mode: AuthMode,
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

#[derive(Debug, Serialize)]
struct OkBody {
    ok: bool,
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

    fn missing_principal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "missing_principal", message)
    }

    fn missing_authentication(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "missing_authentication", message)
    }

    fn invalid_authentication(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "invalid_authentication", message)
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
        .route("/v1/memories", post(put_memory).delete(delete_memory))
        .route("/v1/memories/get", post(get_memory))
        .route("/v1/recall", post(recall))
        .route("/v1/audit", post(audit))
        .route("/v1/vector-health", post(vector_health))
        .route("/v1/vector-repair", post(vector_repair))
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
        auth_mode: state.auth_mode(),
    }))
}

async fn put_memory(
    State(state): State<ServiceState>,
    headers: HeaderMap,
    body: Result<Json<MemoryPutRequest>, JsonRejection>,
) -> Result<Json<loong_memory_core::MemoryRecord>, ApiError> {
    let principal = state.authenticator.authenticate(&headers)?;
    let Json(req) = body.map_err(ApiError::from)?;
    let record = with_engine(state, principal, move |engine, ctx| engine.put(&ctx, &req)).await?;
    Ok(Json(record))
}

async fn get_memory(
    State(state): State<ServiceState>,
    headers: HeaderMap,
    body: Result<Json<MemoryGetRequest>, JsonRejection>,
) -> Result<Json<loong_memory_core::MemoryRecord>, ApiError> {
    let principal = state.authenticator.authenticate(&headers)?;
    let Json(req) = body.map_err(ApiError::from)?;
    let record = with_engine(state, principal, move |engine, ctx| engine.get(&ctx, &req)).await?;
    Ok(Json(record))
}

async fn delete_memory(
    State(state): State<ServiceState>,
    headers: HeaderMap,
    body: Result<Json<MemoryDeleteRequest>, JsonRejection>,
) -> Result<Json<OkBody>, ApiError> {
    let principal = state.authenticator.authenticate(&headers)?;
    let Json(req) = body.map_err(ApiError::from)?;
    with_engine(state, principal, move |engine, ctx| {
        engine.delete(&ctx, &req)
    })
    .await?;
    Ok(Json(OkBody { ok: true }))
}

async fn recall(
    State(state): State<ServiceState>,
    headers: HeaderMap,
    body: Result<Json<RecallRequestBody>, JsonRejection>,
) -> Result<Json<CountEnvelope<HitsBody>>, ApiError> {
    let principal = state.authenticator.authenticate(&headers)?;
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
    let principal = state.authenticator.authenticate(&headers)?;
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

async fn vector_health(
    State(state): State<ServiceState>,
    headers: HeaderMap,
    body: Result<Json<VectorHealthRequestBody>, JsonRejection>,
) -> Result<Json<loong_memory_core::VectorHealthReport>, ApiError> {
    let principal = state.authenticator.authenticate(&headers)?;
    let Json(req) = body.map_err(ApiError::from)?;
    let report = with_engine(state, principal, move |engine, ctx| {
        engine.vector_health(&ctx, &req.namespace, req.invalid_sample_limit)
    })
    .await?;
    Ok(Json(report))
}

async fn vector_repair(
    State(state): State<ServiceState>,
    headers: HeaderMap,
    body: Result<Json<VectorRepairRequestBody>, JsonRejection>,
) -> Result<Json<loong_memory_core::VectorRepairReport>, ApiError> {
    let principal = state.authenticator.authenticate(&headers)?;
    let Json(req) = body.map_err(ApiError::from)?;
    let report = with_engine(state, principal, move |engine, ctx| {
        engine.vector_repair(&ctx, &req.namespace, req.issue_sample_limit, req.apply)
    })
    .await?;
    Ok(Json(report))
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

fn default_invalid_sample_limit() -> usize {
    20
}

fn default_issue_sample_limit() -> usize {
    20
}

fn extract_trusted_header_principal(headers: &HeaderMap) -> Result<String, ApiError> {
    let Some(value) = headers.get(PRINCIPAL_HEADER) else {
        return Err(ApiError::missing_principal(format!(
            "missing required header {PRINCIPAL_HEADER}"
        )));
    };
    let principal = value
        .to_str()
        .map_err(|_| ApiError::missing_principal(format!("invalid header {PRINCIPAL_HEADER}")))?
        .trim()
        .to_owned();
    if principal.is_empty() {
        return Err(ApiError::missing_principal(format!(
            "missing required header {PRINCIPAL_HEADER}"
        )));
    }
    Ok(principal)
}

fn extract_bearer_token(headers: &HeaderMap) -> Result<&str, ApiError> {
    let Some(value) = headers.get(header::AUTHORIZATION) else {
        return Err(ApiError::missing_authentication(
            "missing required header authorization",
        ));
    };
    let authorization = value
        .to_str()
        .map_err(|_| ApiError::invalid_authentication("invalid authorization header"))?
        .trim();
    let mut parts = authorization.split_whitespace();
    let Some(scheme) = parts.next() else {
        return Err(ApiError::invalid_authentication(
            "expected Authorization: Bearer <token>",
        ));
    };
    let Some(token) = parts.next() else {
        return Err(ApiError::invalid_authentication(
            "expected Authorization: Bearer <token>",
        ));
    };
    if parts.next().is_some() || !scheme.eq_ignore_ascii_case("bearer") {
        return Err(ApiError::invalid_authentication(
            "expected Authorization: Bearer <token>",
        ));
    }
    Ok(token)
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

fn load_authenticator(auth_file: Option<&Path>) -> Result<Authenticator> {
    match auth_file {
        Some(path) => {
            let raw = std::fs::read_to_string(path)
                .with_context(|| format!("read auth file {}", path.display()))?;
            let config: StaticAuthConfig = serde_json::from_str(&raw)
                .with_context(|| format!("parse auth file {}", path.display()))?;
            let mut token_to_principal = HashMap::new();
            for token_config in config.tokens {
                let token = token_config.token.trim();
                if token.is_empty() {
                    bail!("auth file {} contains an empty token", path.display());
                }
                if token.chars().any(char::is_whitespace) {
                    bail!(
                        "auth file {} contains token entries with whitespace",
                        path.display()
                    );
                }

                let principal = token_config.principal.trim();
                if principal.is_empty() {
                    bail!("auth file {} contains an empty principal", path.display());
                }
                if token_to_principal
                    .insert(token.to_owned(), principal.to_owned())
                    .is_some()
                {
                    bail!(
                        "auth file {} contains duplicate token entries",
                        path.display()
                    );
                }
            }

            Ok(Authenticator::StaticToken {
                token_to_principal: Arc::new(token_to_principal),
            })
        }
        None => Ok(Authenticator::TrustedHeader),
    }
}
