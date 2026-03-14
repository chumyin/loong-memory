use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Put,
    Get,
    Recall,
    Delete,
    AuditRead,
    Repair,
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

#[derive(Debug, Clone, Default, Deserialize)]
pub struct StaticPolicyConfig {
    #[serde(default)]
    pub namespace_actions: BTreeMap<String, Vec<Action>>,
    #[serde(default)]
    pub principal_namespace_actions: Vec<PrincipalNamespaceActionsConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PrincipalNamespaceActionsConfig {
    pub principal: String,
    pub namespace: String,
    pub actions: Vec<Action>,
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

    pub fn from_config(config: StaticPolicyConfig) -> Self {
        let mut policy = Self::default();
        for (namespace, actions) in config.namespace_actions {
            policy = policy.allow_namespace_actions(namespace, actions);
        }
        for entry in config.principal_namespace_actions {
            policy = policy.allow_principal_namespace_actions(
                entry.principal,
                entry.namespace,
                entry.actions,
            );
        }
        policy
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
