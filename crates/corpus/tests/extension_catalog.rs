use corpus::{CorpusError, ExtensionCatalog};
use fabric::types::admission::RiskLevel;
use fabric::{
    ActivationConstraints, CapabilityId, ExtensionCatalog as _, ExtensionDescriptor, ExtensionId,
    ExtensionKind, ExtensionOrigin, ToolDefinition,
};
use std::sync::Arc;

fn descriptor(kind: ExtensionKind, name: &str, capability: &str) -> ExtensionDescriptor {
    let value = ExtensionDescriptor::new(
        kind,
        name,
        "1.0.0",
        format!("{name} extension"),
        CapabilityId(capability.into()),
        RiskLevel::ReadOnly,
    )
    .unwrap()
    .with_origin(match kind {
        ExtensionKind::Mcp => ExtensionOrigin::Remote {
            server: "search".into(),
        },
        ExtensionKind::Plugin | ExtensionKind::Skill | ExtensionKind::Hook => {
            ExtensionOrigin::FileSystem {
                path: format!("/extensions/{name}"),
            }
        }
        ExtensionKind::Tool => ExtensionOrigin::BuiltIn,
    })
    .with_activation_constraints(ActivationConstraints {
        required_config_flags: vec![format!("extensions.{name}.enabled")],
        ..Default::default()
    });
    if matches!(kind, ExtensionKind::Tool | ExtensionKind::Mcp) {
        value
            .with_tool_definition(ToolDefinition {
                name: capability.into(),
                description: name.into(),
                input_schema: serde_json::json!({"type":"object"}),
            })
            .unwrap()
    } else {
        value
    }
}

#[test]
fn indexes_all_extension_kinds_without_activating_and_snapshots_stably() {
    let catalog = ExtensionCatalog::new([
        descriptor(ExtensionKind::Skill, "review", "skill.review"),
        descriptor(ExtensionKind::Plugin, "git", "plugin.git"),
        descriptor(ExtensionKind::Mcp, "search", "mcp.search"),
        descriptor(ExtensionKind::Hook, "audit", "hook.audit"),
        descriptor(ExtensionKind::Tool, "read", "file.read"),
    ])
    .unwrap();
    let ids: Vec<_> = catalog
        .snapshot()
        .entries
        .into_iter()
        .map(|entry| entry.id.as_str().to_string())
        .collect();
    assert_eq!(
        ids,
        [
            "hook:audit",
            "mcp:search",
            "plugin:git",
            "skill:review",
            "tool:read"
        ]
    );
}

#[test]
fn duplicate_identity_and_executable_capability_conflicts_are_deterministic() {
    let duplicate = ExtensionCatalog::new([
        descriptor(ExtensionKind::Skill, "review", "skill.review"),
        descriptor(ExtensionKind::Skill, "review", "skill.other"),
    ])
    .unwrap_err();
    assert!(matches!(duplicate, CorpusError::DuplicateExtension(id) if id == "skill:review"));

    let conflict = ExtensionCatalog::new([
        descriptor(ExtensionKind::Tool, "read", "file.read"),
        descriptor(ExtensionKind::Mcp, "remote-read", "file.read"),
    ])
    .unwrap_err();
    assert!(
        matches!(conflict, CorpusError::ConflictingCapability { capability, .. } if capability == "file.read")
    );
}

#[test]
fn runtime_skills_and_hooks_are_discovered_before_activation() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("review.md"),
        "# Review\nChecks a result.\n\nReview instructions.",
    )
    .unwrap();
    let mut skills = corpus::SkillLoader::new(root.path().to_path_buf());
    assert_eq!(skills.load_all_enhanced(), 1);
    let mut hooks = corpus::HookRegistry::new(Arc::new(kernel::chronos::TestClock::default()));
    hooks.register(corpus::hook::registry::RegisteredHook {
        name: "audit".into(),
        source: "builtin".into(),
        script_path: None,
        point: fabric::hook::HookPoint::PostTool,
        priority: 10,
    });

    let descriptors = corpus::discover_runtime_extensions(&skills, &hooks).unwrap();
    assert_eq!(descriptors.len(), 2);
    assert!(descriptors
        .iter()
        .any(|entry| entry.kind == ExtensionKind::Skill));
    assert!(descriptors
        .iter()
        .any(|entry| entry.kind == ExtensionKind::Hook));
}

#[test]
fn tool_identity_format_is_kind_colon_local_name() {
    let id = ExtensionId::new(ExtensionKind::Tool, "read").unwrap();
    assert_eq!(id.as_str(), "tool:read");
    assert_eq!(
        ExtensionId::new(ExtensionKind::Tool, "bash_exec")
            .unwrap()
            .as_str(),
        "tool:bash_exec"
    );
    assert_eq!(
        ExtensionId::new(ExtensionKind::Mcp, "search")
            .unwrap()
            .as_str(),
        "mcp:search"
    );
}

#[test]
fn tool_call_identity_uses_extension_kind_tool() {
    let ids = [
        ExtensionId::new(ExtensionKind::Tool, "bash_exec").unwrap(),
        ExtensionId::new(ExtensionKind::Tool, "file_write").unwrap(),
        ExtensionId::new(ExtensionKind::Tool, "file_read").unwrap(),
    ];
    for id in &ids {
        assert!(
            id.as_str().starts_with("tool:"),
            "{} should use tool: prefix",
            id.as_str()
        );
    }
}

#[test]
fn descriptor_without_tool_definition_is_not_executable() {
    let d = descriptor(ExtensionKind::Skill, "review", "skill.review");
    assert!(!d.is_executable());

    let mcp = ExtensionDescriptor::new(
        ExtensionKind::Mcp,
        "search-no-tool",
        "1.0.0",
        "search without tool definition",
        CapabilityId("mcp.search-notool".into()),
        RiskLevel::ReadOnly,
    )
    .unwrap()
    .with_origin(ExtensionOrigin::Remote {
        server: "search".into(),
    });
    assert!(!mcp.is_executable());
}

#[test]
fn empty_capabilities_rejected() {
    let empty = ExtensionDescriptor {
        id: ExtensionId::new(ExtensionKind::Tool, "empty-tool").unwrap(),
        kind: ExtensionKind::Tool,
        version: "1.0.0".into(),
        description: "no capabilities".into(),
        capabilities: vec![],
        origin: ExtensionOrigin::BuiltIn,
        activation: ActivationConstraints::default(),
        risk: RiskLevel::ReadOnly,
        tool_definition: None,
    };
    let err = ExtensionCatalog::new([empty]).unwrap_err();
    assert!(
        matches!(err, CorpusError::InvalidDescriptor(msg) if msg.contains("declares no capabilities"))
    );
}
