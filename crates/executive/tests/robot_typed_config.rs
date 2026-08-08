//! R1 acceptance contracts for layered Robot/Policy configuration.

use executive::composition::config::{
    merge_layers, AppConfig, ConfigLayer, ConfigSource, ConfigSourceKind,
};

fn layer(kind: ConfigSourceKind, locator: &str, text: &str) -> ConfigLayer {
    ConfigLayer::from_toml(ConfigSource::new(kind, locator), text).unwrap()
}

fn robot_config(policy_endpoint: &str) -> String {
    format!(
        r#"
[agent]
harness_kind = "robot"

[integrations.embodiment]
kind = "grpc"
device_id = "kuavo-mujoco-01"
endpoint = "http://127.0.0.1:50051"
connect_timeout_ms = 5000
request_timeout_ms = 10000

[integrations.robot]
scene_version = "kuavo-mujoco/default-v40"
execution_environment = "simulation"
max_retries = 1
max_replans = 1

[integrations.robot.policy]
endpoint = "{policy_endpoint}"
protocol_version = "1.0"
connect_timeout_ms = 5000
request_timeout_ms = 30000
max_proposals = 4

[integrations.robot.perception]
poll_interval_ms = 250
max_devices = 16
"#
    )
}

#[test]
fn robot_policy_layer_override_is_typed_and_provenanced() {
    let loaded = merge_layers([
        layer(
            ConfigSourceKind::System,
            "/etc/aletheon/config.toml",
            &robot_config("http://127.0.0.1:50052"),
        ),
        layer(
            ConfigSourceKind::Environment,
            "environment:ALETHEON_POLICY_ENDPOINT",
            "[integrations.robot.policy]\nendpoint='https://policy.example.test:443'",
        ),
    ])
    .unwrap();

    let resolved = loaded
        .value
        .resolve_robot_config()
        .unwrap()
        .expect("Robot config");
    assert_eq!(resolved.policy.endpoint, "https://policy.example.test:443");
    assert_eq!(resolved.device_id, "kuavo-mujoco-01");
    assert_eq!(resolved.scene_version, "kuavo-mujoco/default-v40");
    assert_eq!(
        resolved.execution_environment,
        fabric::types::embodiment::ExecutionEnvironment::Simulation
    );
    assert_eq!(
        resolved.bridge_protocol_digest,
        hardware::grpc::BRIDGE_PROTOCOL_DIGEST
    );
    assert_eq!(resolved.aletheon_version, env!("CARGO_PKG_VERSION"));
    assert_eq!(resolved.policy.connect_timeout.as_millis(), 5_000);
    assert_eq!(resolved.perception.poll_interval.as_millis(), 250);
    assert_eq!(resolved.perception.max_frame_age.as_millis(), 2_000);
    assert_eq!(resolved.perception.max_frames, 4);
    assert_eq!(resolved.perception.max_total_bytes, 16 * 1024 * 1024);
    assert_eq!(
        resolved.perception.allowed_uri_prefixes,
        vec!["artifact://sha256/"]
    );
    assert_eq!(
        loaded
            .source("integrations.robot.policy.endpoint")
            .unwrap()
            .kind,
        ConfigSourceKind::Environment
    );

    let effective = loaded.effective_view().config.to_string();
    assert!(effective.contains("https://policy.example.test:443"));
    assert!(effective.contains("kuavo-mujoco-01"));
    assert!(effective.contains("kuavo-mujoco/default-v40"));
}

#[test]
fn robot_harness_fails_closed_without_policy_config() {
    let error = merge_layers([layer(
        ConfigSourceKind::User,
        "robot-without-policy",
        r#"
[agent]
harness_kind = "robot"
[integrations.embodiment]
kind = "simulator"
device_id = "sim-01"
[integrations.robot]
scene_version = "scene-v1"
"#,
    )])
    .unwrap_err()
    .to_string();
    assert!(error.contains("integrations.robot.policy"), "{error}");
}

#[test]
fn legacy_harness_hint_without_robot_config_keeps_general_available() {
    let loaded = merge_layers([layer(
        ConfigSourceKind::User,
        "legacy-embodiment-only",
        r#"
[agent]
harness_kind = "robot"
[integrations.embodiment]
kind = "simulator"
device_id = "sim-01"
"#,
    )])
    .unwrap();
    assert!(loaded.value.resolve_robot_config().unwrap().is_none());
}

#[test]
fn linear_harness_does_not_require_robot_configuration() {
    let config = AppConfig::default();
    assert_eq!(
        config.agent.harness_kind,
        cognit::harness::HarnessKind::Linear
    );
    assert!(config.resolve_robot_config().unwrap().is_none());

    let loaded = merge_layers([layer(
        ConfigSourceKind::User,
        "legacy-linear-embodiment",
        r#"
[integrations.embodiment]
kind = "simulator"
device_id = "legacy-sim"
"#,
    )])
    .unwrap();
    assert!(loaded.value.resolve_robot_config().unwrap().is_none());

    let endpoint_override_only = merge_layers([layer(
        ConfigSourceKind::Environment,
        "environment:ALETHEON_POLICY_ENDPOINT",
        "[integrations.robot.policy]\nendpoint='http://127.0.0.1:50052'",
    )])
    .unwrap_err()
    .to_string();
    assert!(
        endpoint_override_only.contains("integrations.embodiment"),
        "{endpoint_override_only}"
    );
}

#[test]
fn complete_robot_integration_is_available_even_with_linear_legacy_hint() {
    let config = robot_config("http://127.0.0.1:50052")
        .replace("harness_kind = \"robot\"", "harness_kind = \"linear\"");
    let loaded =
        merge_layers([layer(ConfigSourceKind::User, "robot-capability", &config)]).unwrap();
    assert!(loaded.value.resolve_robot_config().unwrap().is_some());
}

#[test]
fn invalid_robot_values_are_rejected_before_bootstrap() {
    let cases = [
        (
            "connect_timeout_ms = 5000",
            "connect_timeout_ms = 0",
            "connect_timeout_ms",
        ),
        (
            "device_id = \"kuavo-mujoco-01\"",
            "device_id = \"\"",
            "device_id",
        ),
        (
            "protocol_version = \"1.0\"",
            "protocol_version = \"\"",
            "protocol_version",
        ),
        (
            "endpoint = \"http://127.0.0.1:50052\"",
            "endpoint = \"http://policy.example.test:50052\"",
            "plaintext",
        ),
        (
            "endpoint = \"http://127.0.0.1:50052\"",
            "endpoint = \"https://user:secret@policy.example.test:443\"",
            "userinfo",
        ),
        (
            "max_devices = 16",
            "max_devices = 0",
            "perception.max_devices",
        ),
        (
            "max_devices = 16",
            "max_devices = 16\nmax_frames = 5",
            "perception.max_frames",
        ),
        (
            "max_devices = 16",
            "max_devices = 16\nallowed_uri_prefixes = [\"file://\"]",
            "allowed_uri_prefixes",
        ),
    ];

    for (original, replacement, expected) in cases {
        let config = robot_config("http://127.0.0.1:50052").replacen(original, replacement, 1);
        let error = merge_layers([layer(ConfigSourceKind::Cli, expected, &config)])
            .unwrap_err()
            .to_string();
        assert!(error.contains(expected), "expected {expected} in: {error}");
        assert!(!error.contains("user:secret"), "secret leaked in: {error}");
    }
}

#[test]
fn skill_specific_perception_requirements_are_typed() {
    let config = format!(
        "{}\n[integrations.robot.perception.required_by_skill]\n\"kuavo.visual_inspect\" = [{{ schema = \"camera.rgb\", schema_version = 1 }}]\n",
        robot_config("http://127.0.0.1:50052")
    );
    let loaded = merge_layers([layer(ConfigSourceKind::User, "typed-perception", &config)])
        .expect("typed perception config");
    let resolved = loaded
        .value
        .resolve_robot_config()
        .unwrap()
        .expect("Robot config");
    assert_eq!(
        resolved
            .perception
            .required_by_skill
            .get(&fabric::types::embodiment::SkillId(
                "kuavo.visual_inspect".into()
            )),
        Some(&vec![("camera.rgb".into(), 1)])
    );
}

#[test]
fn simulator_config_cannot_claim_hil_or_real_profile() {
    for environment in ["hil", "real"] {
        let text = robot_config("http://127.0.0.1:50052")
            .replace("kind = \"grpc\"", "kind = \"simulator\"")
            .replace(
                "endpoint = \"http://127.0.0.1:50051\"\nconnect_timeout_ms = 5000\nrequest_timeout_ms = 10000\n",
                "",
            )
            .replace(
                "execution_environment = \"simulation\"",
                &format!("execution_environment = \"{environment}\""),
            );
        let error = merge_layers([layer(ConfigSourceKind::User, "profile-spoof", &text)])
            .unwrap_err()
            .to_string();
        assert!(error.contains("can only use"), "{error}");
    }
}

fn deployment_gate_toml(include_real_evidence: bool) -> String {
    let mut gate = format!(
        r#"
[integrations.robot.deployment_gate]
device_serial = "SN-KUAVO-001"
safety_manifest_digest = "{}"
limits_digest = "{}"
"#,
        "a".repeat(64),
        "b".repeat(64),
    );
    if include_real_evidence {
        gate.push_str(&format!(
            "evidence_digest = \"{}\"\nevidence_expiry_unix_ms = 4102444800000\n",
            "c".repeat(64)
        ));
    }
    gate
}

#[test]
fn hil_and_real_profiles_require_a_pinned_deployment_gate() {
    for environment in ["hil", "real"] {
        let text = robot_config("http://127.0.0.1:50052").replace(
            "execution_environment = \"simulation\"",
            &format!("execution_environment = \"{environment}\""),
        );
        let error = merge_layers([layer(ConfigSourceKind::User, "missing-gate", &text)])
            .unwrap_err()
            .to_string();
        assert!(error.contains("deployment_gate"), "{error}");
    }
}

#[test]
fn simulation_profile_rejects_hil_or_real_gate_material() {
    let text = format!(
        "{}{}",
        robot_config("http://127.0.0.1:50052"),
        deployment_gate_toml(false)
    );
    let error = merge_layers([layer(ConfigSourceKind::User, "sim-with-gate", &text)])
        .unwrap_err()
        .to_string();
    assert!(error.contains("must be omitted"), "{error}");
}

#[test]
fn hil_profile_resolves_pinned_device_and_bridge_digests() {
    let text = format!(
        "{}{}",
        robot_config("http://127.0.0.1:50052").replace(
            "execution_environment = \"simulation\"",
            "execution_environment = \"hil\"",
        ),
        deployment_gate_toml(false)
    );
    let loaded = merge_layers([layer(ConfigSourceKind::User, "hil-gate", &text)]).unwrap();
    let resolved = loaded
        .value
        .resolve_robot_config()
        .unwrap()
        .expect("Robot config");
    let gate = resolved.deployment_gate.expect("HIL gate");
    assert_eq!(gate.device_serial, "SN-KUAVO-001");
    assert_eq!(gate.safety_manifest_digest, "a".repeat(64));
    assert_eq!(gate.limits_digest, "b".repeat(64));
}

#[test]
fn real_profile_requires_non_loopback_tls_and_reviewed_evidence() {
    let base = robot_config("http://127.0.0.1:50052")
        .replace(
            "execution_environment = \"simulation\"",
            "execution_environment = \"real\"",
        )
        .replace("http://127.0.0.1:50051", "https://127.0.0.1:50051");
    let loopback = format!("{base}{}", deployment_gate_toml(true));
    let error = merge_layers([layer(ConfigSourceKind::User, "real-loopback", &loopback)])
        .unwrap_err()
        .to_string();
    assert!(error.contains("non-loopback"), "{error}");

    let real = loopback.replace(
        "https://127.0.0.1:50051",
        "https://bridge.example.test:50051",
    );
    let loaded = merge_layers([layer(ConfigSourceKind::User, "real-gate", &real)]).unwrap();
    let resolved = loaded
        .value
        .resolve_robot_config()
        .unwrap()
        .expect("Robot config");
    let gate = resolved.deployment_gate.expect("real gate");
    assert_eq!(gate.evidence_digest, "c".repeat(64));
    assert_eq!(gate.evidence_expiry_unix_ms, 4_102_444_800_000);
}
