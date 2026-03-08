use loong_memory_core::{Action, PolicyDecision, PolicyEngine, StaticPolicy};

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
