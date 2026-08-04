use fabric::protocol::client::ClientRpcRequest;
use fabric::protocol::extension::{
    ExtensionEnableRequestV1, ExtensionPackageIdRequestV1, ExtensionPackagePathRequestV1,
    McpConnectorManifestV1,
};

#[test]
fn connector_rejects_inline_secret_values() {
    let raw = serde_json::json!({
        "schema_version": 1,
        "id": "aurb.gbrain",
        "transport": {"kind":"streamable_http","url":"http://127.0.0.1:3131/mcp"},
        "bearer_token_env": "Bearer literal-token"
    });
    let parsed = serde_json::from_value::<McpConnectorManifestV1>(raw).unwrap();
    assert!(parsed.validate().is_err());
}

#[test]
fn connector_accepts_secret_reference_and_rejects_url_credentials() {
    let valid = serde_json::from_value::<McpConnectorManifestV1>(serde_json::json!({
        "schema_version": 1,
        "id": "aurb.gbrain",
        "transport": {"kind":"streamable_http","url":"https://localhost:3131/mcp"},
        "bearer_token_env": "AURB_GBRAIN_TOKEN"
    }))
    .unwrap();
    valid.validate().unwrap();

    let with_credentials = serde_json::from_value::<McpConnectorManifestV1>(serde_json::json!({
        "schema_version": 1,
        "id": "aurb.gbrain",
        "transport": {"kind":"sse","url":"https://user:secret@localhost/mcp"}
    }))
    .unwrap();
    assert!(with_credentials.validate().is_err());
}

#[test]
fn stdio_connector_command_must_stay_below_payload() {
    for command in ["bin/server", "../payload/server", "/payload/server"] {
        let parsed = serde_json::from_value::<McpConnectorManifestV1>(serde_json::json!({
            "schema_version": 1,
            "id": "aurb.local",
            "transport": {"kind":"stdio","command":command,"args":[]}
        }))
        .unwrap();
        assert!(parsed.validate().is_err(), "accepted {command}");
    }

    let valid = serde_json::from_value::<McpConnectorManifestV1>(serde_json::json!({
        "schema_version": 1,
        "id": "aurb.local",
        "transport": {"kind":"stdio","command":"payload/bin/server","args":[]}
    }))
    .unwrap();
    valid.validate().unwrap();
}

#[test]
fn extension_enable_serializes_versioned_rpc() {
    let request = ClientRpcRequest::ExtensionEnable(ExtensionEnableRequestV1 {
        schema_version: 1,
        package_id: "aurb.core".into(),
        approve_permissions: true,
    })
    .to_json_rpc(Some(9))
    .unwrap();
    assert_eq!(request["method"], "extension.enable");
    assert_eq!(request["params"]["schema_version"], 1);
    assert_eq!(request["params"]["package_id"], "aurb.core");
}

#[test]
fn extension_lifecycle_methods_have_stable_names() {
    let path = || ExtensionPackagePathRequestV1 {
        schema_version: 1,
        path: "/tmp/package.tar.gz".into(),
        trust_workspace: false,
        approve_permissions: false,
    };
    let id = || ExtensionPackageIdRequestV1 {
        schema_version: 1,
        package_id: "aurb.core".into(),
    };
    let requests = [
        (
            ClientRpcRequest::ExtensionInstall(path()),
            "extension.install",
        ),
        (
            ClientRpcRequest::ExtensionDisable(id()),
            "extension.disable",
        ),
        (
            ClientRpcRequest::ExtensionUpgrade(path()),
            "extension.upgrade",
        ),
        (
            ClientRpcRequest::ExtensionRollback(id()),
            "extension.rollback",
        ),
        (ClientRpcRequest::ExtensionRemove(id()), "extension.remove"),
        (ClientRpcRequest::ExtensionPurge(id()), "extension.purge"),
        (ClientRpcRequest::ExtensionList, "extension.list"),
        (ClientRpcRequest::ExtensionShow(id()), "extension.show"),
        (ClientRpcRequest::ExtensionDoctor(id()), "extension.doctor"),
    ];
    for (request, method) in requests {
        assert_eq!(request.to_json_rpc(Some(1)).unwrap()["method"], method);
    }
}
