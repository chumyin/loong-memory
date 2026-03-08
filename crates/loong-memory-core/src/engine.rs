use std::sync::Arc;

use chrono::Utc;
use serde_json::json;
use uuid::Uuid;

use crate::{
    audit::{AuditEvent, AuditEventKind, AuditSink},
    embed::EmbeddingProvider,
    error::LoongMemoryError,
    model::{
        MemoryDeleteRequest, MemoryGetRequest, MemoryPutRequest, MemoryRecord, RecallHit,
        RecallRequest,
    },
    policy::{Action, PolicyDecision, PolicyEngine},
    store::MemoryStore,
};

#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub max_content_bytes: usize,
    pub max_query_bytes: usize,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            max_content_bytes: 16 * 1024,
            max_query_bytes: 2048,
        }
    }
}

#[derive(Debug, Clone)]
pub struct OperationContext {
    pub principal: String,
}

impl OperationContext {
    pub fn new(principal: impl Into<String>) -> Self {
        Self {
            principal: principal.into(),
        }
    }
}

pub struct MemoryEngine<S: MemoryStore> {
    store: S,
    policy: Arc<dyn PolicyEngine>,
    embedder: Arc<dyn EmbeddingProvider>,
    audit: Arc<dyn AuditSink>,
    config: EngineConfig,
}

impl<S: MemoryStore> MemoryEngine<S> {
    pub fn new(
        store: S,
        policy: Arc<dyn PolicyEngine>,
        embedder: Arc<dyn EmbeddingProvider>,
        audit: Arc<dyn AuditSink>,
        config: EngineConfig,
    ) -> Self {
        Self {
            store,
            policy,
            embedder,
            audit,
            config,
        }
    }

    pub fn put(
        &mut self,
        ctx: &OperationContext,
        req: &MemoryPutRequest,
    ) -> Result<MemoryRecord, LoongMemoryError> {
        self.validate_put(req)?;
        self.enforce(ctx, &req.namespace, Action::Put)?;
        let out = self.store.put(req, self.embedder.as_ref())?;
        self.emit(
            ctx,
            &req.namespace,
            "put",
            AuditEventKind::Write,
            json!({"id": out.id}),
        );
        Ok(out)
    }

    pub fn get(
        &self,
        ctx: &OperationContext,
        req: &MemoryGetRequest,
    ) -> Result<MemoryRecord, LoongMemoryError> {
        self.enforce(ctx, &req.namespace, Action::Get)?;
        let out = self.store.get(req)?;
        self.emit(
            ctx,
            &req.namespace,
            "get",
            AuditEventKind::Read,
            json!({"id": out.id}),
        );
        Ok(out)
    }

    pub fn delete(
        &mut self,
        ctx: &OperationContext,
        req: &MemoryDeleteRequest,
    ) -> Result<(), LoongMemoryError> {
        self.enforce(ctx, &req.namespace, Action::Delete)?;
        self.store.delete(req)?;
        self.emit(
            ctx,
            &req.namespace,
            "delete",
            AuditEventKind::Delete,
            json!({"ok": true}),
        );
        Ok(())
    }

    pub fn recall(
        &self,
        ctx: &OperationContext,
        req: &RecallRequest,
    ) -> Result<Vec<RecallHit>, LoongMemoryError> {
        self.validate_recall(req)?;
        self.enforce(ctx, &req.namespace, Action::Recall)?;
        let out = self.store.recall(req, self.embedder.as_ref())?;
        self.emit(
            ctx,
            &req.namespace,
            "recall",
            AuditEventKind::Recall,
            json!({"hits": out.len()}),
        );
        Ok(out)
    }

    fn enforce(
        &self,
        ctx: &OperationContext,
        namespace: &str,
        action: Action,
    ) -> Result<(), LoongMemoryError> {
        match self.policy.decide(&ctx.principal, namespace, action) {
            PolicyDecision::Allow => {
                self.emit(
                    ctx,
                    namespace,
                    &format!("{action:?}"),
                    AuditEventKind::Allowed,
                    json!({}),
                );
                Ok(())
            }
            PolicyDecision::Deny(reason) => {
                self.emit(
                    ctx,
                    namespace,
                    &format!("{action:?}"),
                    AuditEventKind::Denied,
                    json!({"reason": reason}),
                );
                Err(LoongMemoryError::PolicyDenied(reason))
            }
        }
    }

    fn validate_put(&self, req: &MemoryPutRequest) -> Result<(), LoongMemoryError> {
        if req.namespace.trim().is_empty() {
            return Err(LoongMemoryError::Validation(
                "namespace is required".to_owned(),
            ));
        }
        if req.content.len() > self.config.max_content_bytes {
            return Err(LoongMemoryError::Validation("content too large".to_owned()));
        }
        if !req.metadata.is_object() {
            return Err(LoongMemoryError::Validation(
                "metadata must be a json object".to_owned(),
            ));
        }
        Ok(())
    }

    fn validate_recall(&self, req: &RecallRequest) -> Result<(), LoongMemoryError> {
        if req.namespace.trim().is_empty() {
            return Err(LoongMemoryError::Validation(
                "namespace is required".to_owned(),
            ));
        }
        if req.query.trim().is_empty() {
            return Err(LoongMemoryError::Validation("query is required".to_owned()));
        }
        if req.query.len() > self.config.max_query_bytes {
            return Err(LoongMemoryError::Validation("query too large".to_owned()));
        }
        if req.limit == 0 {
            return Err(LoongMemoryError::Validation("limit must be > 0".to_owned()));
        }
        Ok(())
    }

    fn emit(
        &self,
        ctx: &OperationContext,
        namespace: &str,
        action: &str,
        kind: AuditEventKind,
        detail: serde_json::Value,
    ) {
        self.audit.record(AuditEvent {
            event_id: Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
            principal: ctx.principal.clone(),
            namespace: namespace.to_owned(),
            action: action.to_owned(),
            kind,
            detail,
        });
    }
}
