use cognit::config::AgentAdmissionConfig;

#[test]
fn config_accepts_nonzero_bounded_tree_budget_and_storage_limits() {
    AgentAdmissionConfig::default().validate().unwrap();
    let parsed: cognit::config::AppConfig =
        toml::from_str(include_str!("../../../config/default.toml")).unwrap();
    parsed.agent.admission.validate().unwrap();
}

#[test]
fn config_rejects_zero_overflow_and_child_allowance_above_root() {
    let config = AgentAdmissionConfig {
        max_running_agents: 0,
        ..AgentAdmissionConfig::default()
    };
    assert!(config.validate().is_err());

    let config = AgentAdmissionConfig {
        max_queued_per_root: AgentAdmissionConfig::default().max_agents_per_root + 1,
        ..AgentAdmissionConfig::default()
    };
    assert!(config.validate().is_err());

    let config = AgentAdmissionConfig {
        max_child_tokens: AgentAdmissionConfig::default().root_max_tokens + 1,
        ..AgentAdmissionConfig::default()
    };
    assert!(config.validate().is_err());

    let config = AgentAdmissionConfig {
        root_max_cost_micro: Some(10),
        max_child_cost_micro: Some(11),
        ..AgentAdmissionConfig::default()
    };
    assert!(config.validate().is_err());
}

#[test]
fn config_rejects_finite_child_cost_under_unbounded_root_representation() {
    let config = AgentAdmissionConfig {
        root_max_cost_micro: None,
        max_child_cost_micro: Some(1),
        ..AgentAdmissionConfig::default()
    };
    assert!(config.validate().is_err());
}
