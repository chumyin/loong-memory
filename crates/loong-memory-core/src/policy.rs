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
    allow_namespace: BTreeMap<String, BTreeSet<Action>>,
    allow_principal_namespace: BTreeMap<(String, String), BTreeSet<Action>>,
}

impl StaticPolicy {
    pub fn allow_namespace_actions(
        mut self,
        namespace: impl Into<String>,
        actions: impl IntoIterator<Item = Action>,
    ) -> Self {
        self.allow_namespace
            .entry(namespace.into())
            .or_default()
            .extend(actions);
        self
    }

    pub fn allow_principal_namespace_actions(
        mut self,
        principal: impl Into<String>,
        namespace: impl Into<String>,
        actions: impl IntoIterator<Item = Action>,
    ) -> Self {
        self.allow_principal_namespace
            .entry((principal.into(), namespace.into()))
            .or_default()
            .extend(actions);
        self
    }
}

impl PolicyEngine for StaticPolicy {
    fn decide(&self, principal: &str, namespace: &str, action: Action) -> PolicyDecision {
        if self
            .allow_principal_namespace
            .get(&(principal.to_owned(), namespace.to_owned()))
            .map(|s| s.contains(&action))
            .unwrap_or(false)
        {
            return PolicyDecision::Allow;
        }
        if self
            .allow_namespace
            .get(namespace)
            .map(|s| s.contains(&action))
            .unwrap_or(false)
        {
            PolicyDecision::Allow
        } else {
            PolicyDecision::Deny(format!(
                "principal={principal} namespace={namespace} action={action:?} not allowed"
            ))
        }
    }
}
