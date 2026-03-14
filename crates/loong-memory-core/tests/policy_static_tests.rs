use loong_memory_core::{Action, PolicyDecision, PolicyEngine, StaticPolicy, StaticPolicyConfig};
use serde_json::json;

#[test]
fn static_policy_supports_principal_scoped_permissions() {
    let policy = StaticPolicy::default()
        .allow_namespace_actions("team-a", [Action::Get])
        .allow_principal_namespace_actions("alice", "team-a", [Action::Put, Action::Delete]);

    assert_eq!(
        policy.decide("alice", "team-a", Action::Put),
        PolicyDecision::Allow
    );
    assert_eq!(
        policy.decide("alice", "team-a", Action::Delete),
        PolicyDecision::Allow
    );

    assert_eq!(
        policy.decide("bob", "team-a", Action::Get),
        PolicyDecision::Allow
    );
    assert!(matches!(
        policy.decide("bob", "team-a", Action::Put),
        PolicyDecision::Deny(_)
    ));
}

#[test]
fn static_policy_is_deny_by_default() {
    let policy = StaticPolicy::default();
    assert!(matches!(
        policy.decide("anyone", "unknown", Action::Get),
        PolicyDecision::Deny(_)
    ));
}

#[test]
fn static_policy_config_supports_snake_case_actions() {
    let config: StaticPolicyConfig = serde_json::from_value(json!({
        "namespace_actions": {
            "shared": ["get", "recall"]
        },
        "principal_namespace_actions": [
            {
                "principal": "operator",
                "namespace": "shared",
                "actions": ["audit_read", "repair"]
            }
        ]
    }))
    .expect("parse static policy config");

    let policy = StaticPolicy::from_config(config);

    assert_eq!(
        policy.decide("operator", "shared", Action::AuditRead),
        PolicyDecision::Allow
    );
    assert_eq!(
        policy.decide("operator", "shared", Action::Repair),
        PolicyDecision::Allow
    );
    assert_eq!(
        policy.decide("guest", "shared", Action::Get),
        PolicyDecision::Allow
    );
    assert!(matches!(
        policy.decide("guest", "shared", Action::Repair),
        PolicyDecision::Deny(_)
    ));
}
