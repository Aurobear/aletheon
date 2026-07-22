use std::path::Path;

use executive::core::config::{
    merge_layers, schema, AppConfig, ConfigLayer, ConfigSource, ConfigSourceKind,
    EnvironmentCredentialResolver, Transport,
};

fn layer(kind: ConfigSourceKind, locator: &str, text: &str) -> ConfigLayer {
    ConfigLayer::from_toml(ConfigSource::new(kind, locator), text).unwrap()
}

#[test]
fn defaults_system_user_project_environment_and_cli_have_total_precedence() {
    let loaded = merge_layers([
        layer(
            ConfigSourceKind::System,
            "/etc/aletheon/config.toml",
            "[agent]\ndefault_model='system'",
        ),
        layer(
            ConfigSourceKind::User,
            "~/.aletheon/config.toml",
            "[agent]\ndefault_model='user'",
        ),
        layer(
            ConfigSourceKind::Project,
            "/repo/.aletheon/config.toml",
            "[agent]\ndefault_model='project'",
        ),
        layer(
            ConfigSourceKind::Environment,
            "ALETHEON__AGENT__DEFAULT_MODEL",
            "[agent]\ndefault_model='environment'",
        ),
        layer(
            ConfigSourceKind::Cli,
            "--agent.default-model",
            "[agent]\ndefault_model='cli'",
        ),
    ])
    .unwrap();

    assert_eq!(loaded.value.agent.default_model.as_deref(), Some("cli"));
    assert_eq!(
        loaded.source("agent.default_model").unwrap().kind,
        ConfigSourceKind::Cli
    );
    assert_eq!(
        loaded.source("agent.max_tokens").unwrap().kind,
        ConfigSourceKind::Default
    );
}

#[test]
fn project_sandbox_profiles_are_additive_and_cannot_override_trusted_names() {
    let loaded = merge_layers([
        layer(
            ConfigSourceKind::User,
            "~/.aletheon/config.toml",
            r#"
[sandbox_profiles]
default_profile = "locked"
[sandbox_profiles.profiles.locked]
extends = "strict"
deny = ["/trusted/secret"]
"#,
        ),
        layer(
            ConfigSourceKind::Project,
            "/repo/.aletheon/config.toml",
            r#"
[sandbox_profiles]
default_profile = "project-only"
[sandbox_profiles.profiles.locked]
extends = "workspace"
deny = []
[sandbox_profiles.profiles.project-only]
extends = "read-only"
deny = ["/project/secret"]
"#,
        ),
    ])
    .unwrap();

    assert_eq!(loaded.value.sandbox_profiles.default_profile, "locked");
    let locked = &loaded.value.sandbox_profiles.profiles["locked"];
    assert_eq!(locked.extends.as_deref(), Some("strict"));
    assert_eq!(locked.deny, vec!["/trusted/secret"]);
    assert!(loaded
        .value
        .sandbox_profiles
        .profiles
        .contains_key("project-only"));
}

#[test]
fn project_cannot_grant_itself_network_authority() {
    let loaded = merge_layers([
        layer(
            ConfigSourceKind::User,
            "~/.aletheon/config.toml",
            "[network_policy]\ndefault_action='deny'",
        ),
        layer(
            ConfigSourceKind::Project,
            "/repo/.aletheon/config.toml",
            "[network_policy]\ndefault_action='allow'\nallow_dns=true",
        ),
    ])
    .unwrap();

    assert_eq!(
        loaded.value.network_policy.default_action,
        fabric::network_policy::NetworkDefaultAction::Deny
    );
    assert!(!loaded.value.network_policy.allow_dns);
    assert_ne!(
        loaded.source("network_policy.default_action").unwrap().kind,
        ConfigSourceKind::Project
    );
}

#[test]
fn validation_errors_name_the_responsible_source_and_reject_unknown_or_invalid_values() {
    let unknown = ConfigLayer::from_toml(
        ConfigSource::new(ConfigSourceKind::Project, "/repo/.aletheon/config.toml"),
        "[agent]\nunknown_switch=true",
    )
    .unwrap_err()
    .to_string();
    assert!(unknown.contains("/repo/.aletheon/config.toml"), "{unknown}");
    assert!(unknown.contains("unknown field"), "{unknown}");

    let invalid = ConfigLayer::from_toml(
        ConfigSource::new(ConfigSourceKind::Cli, "--agent.max-tokens"),
        "[agent]\nmax_tokens='many'",
    )
    .unwrap_err()
    .to_string();
    assert!(invalid.contains("--agent.max-tokens"), "{invalid}");
    assert!(invalid.contains("invalid type"), "{invalid}");
}

#[test]
fn effective_diagnostics_redact_secret_values_and_provenance_debug_never_renders_values() {
    let loaded = merge_layers([layer(
        ConfigSourceKind::User,
        "~/.aletheon/config.toml",
        "[[providers]]\nname='primary'\nbase_url='https://example.invalid'\napi_key='top-secret'",
    )])
    .unwrap();
    let rendered = serde_json::to_string(&loaded.redacted_effective_values()).unwrap();
    assert!(!rendered.contains("top-secret"));
    assert!(rendered.contains("<redacted>"));
    assert!(!format!("{:?}", loaded.provenance).contains("top-secret"));
}

#[test]
fn checked_in_schema_is_deterministic() {
    let generated = schema::generated_schema_json();
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../config/schema/aletheon-config.schema.json");
    if std::env::var_os("UPDATE_CONFIG_SCHEMA").is_some() {
        std::fs::write(&path, &generated).unwrap();
    }
    let checked_in = std::fs::read_to_string(path).unwrap();
    assert_eq!(checked_in, generated);
    assert!(!checked_in.contains("top-secret"));
}

#[test]
fn legacy_channel_and_coding_keys_decode_to_canonical_owned_types() {
    let config: AppConfig = toml::from_str(
        r#"
[telegram]
enabled = false
poll_timeout_secs = 12

[pi_runtime]
enabled = false
json_protocol_version = 3
"#,
    )
    .unwrap();
    let _: &executive::composition::config::TelegramChannelConfig = &config.telegram;
    let _: &executive::composition::config::CodingRuntimeConfig = &config.pi_runtime;
    assert_eq!(config.telegram.poll_timeout_secs, 12);
    assert_eq!(config.pi_runtime.json_protocol_version, 3);
}

#[test]
fn legacy_supplemental_memory_keys_are_read_but_never_reemitted() {
    let memory: executive::composition::config::MemoryConfig = toml::from_str(
        r#"
backend = "sqlite"
data_dir = "/tmp/memory"
[gbrain]
enabled = true
server_name = "deployment-instance"
"#,
    )
    .unwrap();
    assert!(memory.supplemental.enabled);
    let rendered = toml::to_string(&memory).unwrap();
    assert!(rendered.contains("[supplemental]"));
    assert!(!rendered.contains("[gbrain]"));

    let quotas: cognit::config::DeploymentQuotaConfig = toml::from_str(
        "gbrain_spool_bytes=1024\ngbrain_spool_soft_bytes=512\ngbrain_spool_items=4\n",
    )
    .unwrap();
    assert_eq!(quotas.supplemental_spool_bytes, 1024);
    let rendered = toml::to_string(&quotas).unwrap();
    assert!(rendered.contains("supplemental_spool_bytes"));
    assert!(!rendered.contains("gbrain_spool_bytes"));
}

#[test]
fn checked_in_leju_deepseek_uses_the_openai_transport() {
    for relative_path in [
        "../../config/default.toml",
        "../../config/production.toml.example",
    ] {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative_path);
        let config = AppConfig::from_file(&path).unwrap();
        let provider = config
            .providers
            .iter()
            .find(|provider| provider.name == "leju")
            .unwrap_or_else(|| panic!("LejuRobot provider must exist in {}", path.display()));
        assert_eq!(provider.transport, Transport::Openai);
        assert!(provider
            .models
            .iter()
            .any(|model| model == "deepseek/deepseek-v4-pro"));
        assert_eq!(config.agent.default_provider.as_deref(), Some("leju"));
        assert_eq!(
            config.agent.default_model.as_deref(),
            Some("deepseek/deepseek-v4-pro")
        );
    }
}

#[test]
fn enabled_integration_preflight_reports_source_and_missing_typed_path() {
    let loaded = merge_layers([layer(
        ConfigSourceKind::User,
        "~/.aletheon/config.toml",
        "[deployment.integrations]\ngoogle=true",
    )])
    .unwrap();

    let diagnostic = loaded
        .preflight_integrations(&EnvironmentCredentialResolver)
        .unwrap_err()
        .to_string();
    assert!(diagnostic.contains("google=user"), "{diagnostic}");
    assert!(
        diagnostic.contains("integrations.google.client_id"),
        "{diagnostic}"
    );
    assert!(!diagnostic.contains("~/.aletheon"), "{diagnostic}");
}
