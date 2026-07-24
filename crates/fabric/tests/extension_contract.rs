//! Serde compatibility tests for extension types.
//!
//! These tests protect the serialized shape of extension metadata from accidental
//! breakage. Any change to `#[serde(...)]` attributes or field types in
//! `crates/fabric/src/types/extension.rs` or `crates/fabric/src/types/admission.rs`
//! must pass all of the following assertions.

use fabric::types::admission::RiskLevel;
use fabric::ActivationConstraints;
use fabric::CapabilityId;
use fabric::ExtensionDescriptor;
use fabric::ExtensionId;
use fabric::ExtensionKind;
use fabric::ExtensionOrigin;
use fabric::ExtensionSnapshot;
use fabric::ToolDefinition;

use serde_json::from_str;
use serde_json::json;
use serde_json::to_string;
use serde_json::to_value;

// ---------------------------------------------------------------------------
// 1. ExtensionId round-trip
// ---------------------------------------------------------------------------

/// Protects that `ExtensionId` (a newtype `String`) serializes as a bare JSON
/// string and round-trips through serde_json without loss.
#[test]
fn extension_id_serde_round_trip() {
    let cases: &[(&str, ExtensionKind, &str)] = &[
        ("tool:read_file", ExtensionKind::Tool, "read_file"),
        ("skill:my-skill", ExtensionKind::Skill, "my-skill"),
        ("mcp:search", ExtensionKind::Mcp, "search"),
    ];

    for &(expected_str, kind, local_name) in cases {
        let id = ExtensionId::new(kind, local_name).unwrap();
        assert_eq!(id.as_str(), expected_str);

        let json = to_string(&id).unwrap();
        // Newtype string → quoted JSON string.
        assert_eq!(json, format!("\"{expected_str}\""));

        let roundtripped: ExtensionId = from_str(&json).unwrap();
        assert_eq!(roundtripped, id);
    }
}

// ---------------------------------------------------------------------------
// 2. ExtensionKind snake_case serialization
// ---------------------------------------------------------------------------

/// Protects the `#[serde(rename_all = "snake_case")]` contract: every variant
/// serializes to its lowercase `snake_case` form. Downstream consumers (policy
/// engine, protocol clients) depend on these exact string values.
#[test]
fn extension_kind_serializes_snake_case() {
    let cases: &[(ExtensionKind, &str)] = &[
        (ExtensionKind::Tool, "tool"),
        (ExtensionKind::Skill, "skill"),
        (ExtensionKind::Hook, "hook"),
        (ExtensionKind::Plugin, "plugin"),
        (ExtensionKind::Mcp, "mcp"),
    ];

    for (kind, expected) in cases {
        let json = to_value(kind).unwrap();
        assert_eq!(
            json,
            json!(expected),
            "ExtensionKind::{kind:?} must serialize to \"{expected}\""
        );

        // Round-trip from the serialized form.
        let roundtripped: ExtensionKind = from_str(&format!("\"{expected}\"")).unwrap();
        assert_eq!(roundtripped, *kind);
    }
}

// ---------------------------------------------------------------------------
// 3. ExtensionOrigin adjacently tagged serialization
// ---------------------------------------------------------------------------

/// Protects the `#[serde(tag = "origin", rename_all = "snake_case")]` contract.
/// The `"origin"` discriminator field uses `snake_case` variant names ("built_in",
/// not "BuiltIn"). Downstream consumers parse this format to determine provenance.
#[test]
fn extension_origin_tagged_serialization() {
    // --- BuiltIn ---
    let built_in = ExtensionOrigin::BuiltIn;
    let json = to_value(&built_in).unwrap();
    assert_eq!(
        json,
        json!({"origin": "built_in"}),
        "BuiltIn must serialize with tag origin=built_in and NO extra fields"
    );
    let rt: ExtensionOrigin = from_str(r#"{"origin":"built_in"}"#).unwrap();
    assert_eq!(rt, built_in);

    // --- FileSystem { path } ---
    let fs = ExtensionOrigin::FileSystem { path: "/x".into() };
    let json = to_value(&fs).unwrap();
    assert_eq!(
        json,
        json!({"origin": "file_system", "path": "/x"}),
        "FileSystem must include origin=file_system and path field"
    );
    let rt: ExtensionOrigin = from_str(r#"{"origin":"file_system","path":"/x"}"#).unwrap();
    assert_eq!(rt, fs);

    // --- Package { package } ---
    let pkg = ExtensionOrigin::Package {
        package: "pkg".into(),
    };
    let json = to_value(&pkg).unwrap();
    assert_eq!(
        json,
        json!({"origin": "package", "package": "pkg"}),
        "Package must include origin=package and package field"
    );
    let rt: ExtensionOrigin = from_str(r#"{"origin":"package","package":"pkg"}"#).unwrap();
    assert_eq!(rt, pkg);

    // --- Remote { server } ---
    let remote = ExtensionOrigin::Remote {
        server: "srv".into(),
    };
    let json = to_value(&remote).unwrap();
    assert_eq!(
        json,
        json!({"origin": "remote", "server": "srv"}),
        "Remote must include origin=remote and server field"
    );
    let rt: ExtensionOrigin = from_str(r#"{"origin":"remote","server":"srv"}"#).unwrap();
    assert_eq!(rt, remote);
}

// ---------------------------------------------------------------------------
// 4. ExtensionDescriptor round-trip without tool_definition
// ---------------------------------------------------------------------------

/// Protects the full descriptor shape for non-executable extensions (e.g. Skills
/// and Hooks) that carry `tool_definition: None`. Every struct field must survive
/// a JSON round-trip unchanged.
#[test]
fn extension_descriptor_round_trip_without_tool() {
    let descriptor = ExtensionDescriptor {
        id: ExtensionId::new(ExtensionKind::Skill, "summary-skill").unwrap(),
        kind: ExtensionKind::Skill,
        version: "0.3.1".into(),
        description: "Summarizes input text.".into(),
        capabilities: vec![CapabilityId("summary".into())],
        origin: ExtensionOrigin::BuiltIn,
        activation: ActivationConstraints {
            allowed_agents: vec!["orchestrator".into()],
            required_config_flags: vec![],
            requires_approval: false,
        },
        risk: RiskLevel::ReadOnly,
        tool_definition: None,
    };

    let json = to_string(&descriptor).unwrap();
    let roundtripped: ExtensionDescriptor = from_str(&json).unwrap();

    assert_eq!(roundtripped.id, descriptor.id);
    assert_eq!(roundtripped.kind, descriptor.kind);
    assert_eq!(roundtripped.version, descriptor.version);
    assert_eq!(roundtripped.description, descriptor.description);
    assert_eq!(roundtripped.capabilities, descriptor.capabilities);
    assert_eq!(roundtripped.origin, descriptor.origin);
    assert_eq!(roundtripped.activation, descriptor.activation);
    assert_eq!(roundtripped.risk, descriptor.risk);
    assert!(roundtripped.tool_definition.is_none());

    // Non-executable Skill should not report as executable.
    assert!(!roundtripped.is_executable());
}

// ---------------------------------------------------------------------------
// 5. ExtensionDescriptor round-trip with tool_definition
// ---------------------------------------------------------------------------

/// Protects the full descriptor shape for executable extensions (Tools, Mcp) that
/// carry a `tool_definition`. The embedded `ToolDefinition` (name, description,
/// input_schema) must round-trip without loss.
#[test]
fn extension_descriptor_round_trip_with_tool() {
    let descriptor = ExtensionDescriptor {
        id: ExtensionId::new(ExtensionKind::Tool, "read_file").unwrap(),
        kind: ExtensionKind::Tool,
        version: "1.0.0".into(),
        description: "Reads a file from disk.".into(),
        capabilities: vec![CapabilityId("read_file".into())],
        origin: ExtensionOrigin::Package {
            package: "core-tools".into(),
        },
        activation: ActivationConstraints {
            allowed_agents: vec!["coder".into()],
            required_config_flags: vec!["enable-fs".into()],
            requires_approval: true,
        },
        risk: RiskLevel::ReadOnly,
        tool_definition: Some(ToolDefinition {
            name: "read_file".into(),
            description: "Read a file from the filesystem".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" }
                },
                "required": ["path"]
            }),
        }),
    };

    let json = to_string(&descriptor).unwrap();
    let roundtripped: ExtensionDescriptor = from_str(&json).unwrap();

    assert_eq!(roundtripped.id, descriptor.id);
    assert_eq!(roundtripped.kind, descriptor.kind);
    assert_eq!(roundtripped.version, descriptor.version);
    assert_eq!(roundtripped.description, descriptor.description);
    assert_eq!(roundtripped.capabilities, descriptor.capabilities);
    assert_eq!(roundtripped.origin, descriptor.origin);
    assert_eq!(roundtripped.activation, descriptor.activation);
    assert_eq!(roundtripped.risk, descriptor.risk);

    let td = roundtripped.tool_definition.as_ref().unwrap();
    assert_eq!(td.name, "read_file");
    assert_eq!(td.description, "Read a file from the filesystem");
    assert_eq!(
        td.input_schema,
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" }
            },
            "required": ["path"]
        })
    );

    // Executable Tool with a tool_definition should report as executable.
    assert!(roundtripped.is_executable());
}

// ---------------------------------------------------------------------------
// 6. ExtensionSnapshot serialization
// ---------------------------------------------------------------------------

/// Protects `ExtensionSnapshot` which wraps `Vec<ExtensionDescriptor>`. Multiple
/// entries must survive a JSON round-trip with all fields intact.
#[test]
fn extension_snapshot_serialization() {
    let snapshot = ExtensionSnapshot {
        entries: vec![
            ExtensionDescriptor::new(
                ExtensionKind::Tool,
                "read_file",
                "1.0.0",
                "Reads a file from disk.",
                CapabilityId("read_file".into()),
                RiskLevel::ReadOnly,
            )
            .unwrap(),
            ExtensionDescriptor::new(
                ExtensionKind::Skill,
                "code-review",
                "2.1.0",
                "Reviews code changes.",
                CapabilityId("code_review".into()),
                RiskLevel::Sandboxed,
            )
            .unwrap(),
        ],
    };

    let json = to_string(&snapshot).unwrap();
    let roundtripped: ExtensionSnapshot = from_str(&json).unwrap();

    assert_eq!(roundtripped.entries.len(), 2);
    assert_eq!(roundtripped.entries[0].id, snapshot.entries[0].id);
    assert_eq!(roundtripped.entries[1].id, snapshot.entries[1].id);
    assert_eq!(roundtripped.entries[0].kind, ExtensionKind::Tool);
    assert_eq!(roundtripped.entries[1].kind, ExtensionKind::Skill);
}

// ---------------------------------------------------------------------------
// 7. Unknown ExtensionKind deserialization behavior
// ---------------------------------------------------------------------------

/// Protects the current deserialization behaviour for unknown `ExtensionKind`
/// variants. The enum uses standard serde derive (no `#[serde(untagged)]` or
/// `#[serde(other)]`), so an unrecognised string returns a deserialization
/// `Err`. If this behaviour intentionally changes, this test must be
/// updated to document the new contract.
#[test]
fn unknown_extension_kind_deserialization_behavior() {
    let result: Result<ExtensionKind, _> = from_str("\"unknown_variant\"");
    assert!(
        result.is_err(),
        "Deserializing an unknown ExtensionKind variant must return Err \
         (standard serde behaviour for enum without #[serde(other)])"
    );

    // Also confirm that a structurally valid but unknown value fails
    // when embedded inside a larger structure.
    let bad_descriptor_json = json!({
        "id": "skill:test",
        "kind": "not_a_real_kind",
        "version": "1.0.0",
        "description": "test",
        "capabilities": ["test"],
        "origin": {"origin": "built_in"},
        "activation": {
            "allowed_agents": [],
            "required_config_flags": [],
            "requires_approval": false
        },
        "risk": "ReadOnly",
        "tool_definition": null
    })
    .to_string();

    let descriptor_result: Result<ExtensionDescriptor, _> = from_str(&bad_descriptor_json);
    assert!(
        descriptor_result.is_err(),
        "Deserializing an ExtensionDescriptor with an unknown kind must fail"
    );
}

// ---------------------------------------------------------------------------
// 8. Compatibility projections: ExtensionKind → AssetKind
// ---------------------------------------------------------------------------

/// Protects the `From<ExtensionKind> for AssetKind` mapping so Phase 2+
/// can safely project legacy extension kinds into the new asset model.
#[test]
fn extension_kind_to_asset_kind_projection() {
    use fabric::types::extension_asset::AssetKind;
    assert_eq!(AssetKind::from(ExtensionKind::Skill), AssetKind::Skill);
    assert_eq!(AssetKind::from(ExtensionKind::Hook), AssetKind::Hook);
    assert_eq!(AssetKind::from(ExtensionKind::Mcp), AssetKind::Connector);
    assert_eq!(AssetKind::from(ExtensionKind::Tool), AssetKind::Executable);
    assert_eq!(
        AssetKind::from(ExtensionKind::Plugin),
        AssetKind::Executable
    );
}

// ---------------------------------------------------------------------------
// 9. Compatibility projection: ExtensionOrigin → AssetOrigin
// ---------------------------------------------------------------------------

#[test]
fn extension_origin_to_asset_origin_projection() {
    use fabric::types::extension_asset::AssetOrigin;
    let builtin = AssetOrigin::from(ExtensionOrigin::BuiltIn);
    assert_eq!(builtin, AssetOrigin::BuiltIn);

    let fs = AssetOrigin::from(ExtensionOrigin::FileSystem {
        path: "/ext".into(),
    });
    assert_eq!(
        fs,
        AssetOrigin::FileSystem {
            path: "/ext".into(),
        }
    );
}

// ---------------------------------------------------------------------------
// 10. Compatibility projection: ExtensionDescriptor → AssetDescriptor
// ---------------------------------------------------------------------------

#[test]
fn extension_descriptor_to_asset_descriptor_projection() {
    use fabric::types::extension_asset::{AssetDescriptor, AssetKind};
    let old = ExtensionDescriptor::new(
        ExtensionKind::Skill,
        "review",
        "1.0.0",
        "Reviews code.",
        CapabilityId("skill.review".into()),
        RiskLevel::ReadOnly,
    )
    .unwrap();
    let new = AssetDescriptor::from(&old);
    assert_eq!(new.kind, AssetKind::Skill);
    assert_eq!(new.version, "1.0.0");
    assert!(!new.declared_capabilities.is_empty());
}

// ---------------------------------------------------------------------------
// 11. New type serde: PackageId
// ---------------------------------------------------------------------------

#[test]
fn package_id_serde_transparent() {
    use fabric::types::extension_package::PackageId;
    let id = PackageId("test.minimal".into());
    let json = to_string(&id).unwrap();
    assert_eq!(json, "\"test.minimal\"");
    let rt: PackageId = from_str(&json).unwrap();
    assert_eq!(rt, id);
}

// ---------------------------------------------------------------------------
// 12. New type serde: AssetKind
// ---------------------------------------------------------------------------

#[test]
fn asset_kind_serde_snake_case() {
    use fabric::types::extension_asset::AssetKind;
    let cases = [
        (AssetKind::Skill, "skill"),
        (AssetKind::Hook, "hook"),
        (AssetKind::AgentProfile, "agent_profile"),
        (AssetKind::Connector, "connector"),
        (AssetKind::Executable, "executable"),
    ];
    for (kind, expected) in cases {
        let json = to_string(&kind).unwrap();
        assert_eq!(json, format!("\"{expected}\""));
    }
}

// ---------------------------------------------------------------------------
// 13. New type serde: ActivationState
// ---------------------------------------------------------------------------

#[test]
fn activation_state_serde() {
    use fabric::types::extension_state::ActivationState;
    let cases = [
        (ActivationState::Discovered, "\"discovered\""),
        (ActivationState::Active, "\"active\""),
        (ActivationState::Disabled, "\"disabled\""),
    ];
    for (state, expected) in cases {
        assert_eq!(to_string(&state).unwrap(), expected);
    }
}

// ---------------------------------------------------------------------------
// 14. New type serde: HealthState
// ---------------------------------------------------------------------------

#[test]
fn health_state_serde() {
    use fabric::types::extension_state::HealthState;
    assert_eq!(to_string(&HealthState::Healthy).unwrap(), "\"healthy\"");
    let degraded = HealthState::Degraded {
        failures: vec!["timeout".into()],
    };
    let json = to_string(&degraded).unwrap();
    assert!(json.contains("degraded"));
    assert!(json.contains("timeout"));
}
