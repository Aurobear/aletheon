use std::path::Path;

use executive::core::config::{merge_layers, schema, ConfigLayer, ConfigSource, ConfigSourceKind};

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
