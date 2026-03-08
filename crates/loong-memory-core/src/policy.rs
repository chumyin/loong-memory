use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Action {
    Put,
    Get,
    Recall,
    Delete,
    AuditRead,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyDecision {
    Allow,
    Deny(String),
}

pub trait PolicyEngine: Send + Sync {
    fn decide(&self, principal: &str, namespace: &str, action: Action) -> PolicyDecision;
}

#[derive(Debug, Default)]
pub struct AllowAllPolicy;

impl PolicyEngine for AllowAllPolicy {
    fn decide(&self, _principal: &str, _namespace: &str, _action: Action) -> PolicyDecision {
        PolicyDecision::Allow
    }
}

#[derive(Debug, Default)]
pub struct StaticPolicy {
    allow: BTreeMap<String, BTreeSet<Action>>,
}

impl StaticPolicy {
    pub fn allow_namespace_actions(
        mut self,
        namespace: impl Into<String>,
        actions: impl IntoIterator<Item = Action>,
    ) -> Self {
        self.allow
            .entry(namespace.into())
            .or_default()
            .extend(actions);
        self
    }
}

impl PolicyEngine for StaticPolicy {
    fn decide(&self, _principal: &str, namespace: &str, action: Action) -> PolicyDecision {
        if self
            .allow
            .get(namespace)
            .map(|s| s.contains(&action))
            .unwrap_or(false)
        {
            PolicyDecision::Allow
        } else {
            PolicyDecision::Deny(format!(
                "namespace={namespace} action={action:?} not allowed"
            ))
        }
    }
}
