use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use corpus::tools::tools::skill_tools::SharedSkills;
use executive::application::agent_control::AgentRuntimeRegistry;
use executive::application::extension_coordinator::ExtensionCoordinator;
use executive::application::extension_snapshot::{ExtensionRuntimeView, ExtensionSnapshotCompiler};
use executive::application::inference_port::{CoreInferenceRequest, InferenceError, InferencePort};
use executive::composition::config::ExecutiveConfig;
use executive::host::daemon::bootstrap::extension_publisher::{
    DaemonExtensionRuntimePublisher, PackageProfileRuntime,
};
use executive::AgentProfileRegistry;
use fabric::{
    InferenceUsage, LlmProvider, LlmResponse, LlmStream, Message, StopReason, ToolDefinition,
};
use flate2::{write::GzEncoder, Compression};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tokio::sync::Mutex;

struct NoopInference;

#[async_trait]
impl InferencePort for NoopInference {
    async fn complete(&self, _: CoreInferenceRequest) -> Result<LlmResponse, InferenceError> {
        unreachable!("profile composition must not perform inference")
    }

    async fn stream(&self, _: CoreInferenceRequest) -> Result<LlmStream, InferenceError> {
        Ok(Box::pin(futures::stream::empty()))
    }
}

struct NoopLlm;

#[async_trait]
impl LlmProvider for NoopLlm {
    async fn complete(&self, _: &[Message], _: &[ToolDefinition]) -> anyhow::Result<LlmResponse> {
        Ok(LlmResponse {
            content: Vec::new(),
            stop_reason: StopReason::EndTurn,
            usage: InferenceUsage::default(),
        })
    }

    async fn complete_stream(
        &self,
        _: &[Message],
        _: &[ToolDefinition],
    ) -> anyhow::Result<LlmStream> {
        Ok(Box::pin(futures::stream::empty()))
    }

    fn name(&self) -> &str {
        "test-model"
    }

    fn max_context_length(&self) -> usize {
        32_768
    }
}

fn archive(root: &Path, name: &str, files: &[(&str, &[u8])]) -> PathBuf {
    let source = root.join(format!("source-{name}"));
    fs::create_dir_all(&source).unwrap();
    let mut checksums = String::new();
    for (path, body) in files {
        let destination = source.join(path);
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::write(&destination, body).unwrap();
        checksums.push_str(&format!("{:x}  {path}\n", Sha256::digest(body)));
    }
    fs::write(source.join("checksums.sha256"), checksums).unwrap();
    let output = root.join(format!("{name}.tar.gz"));
    let encoder = GzEncoder::new(fs::File::create(&output).unwrap(), Compression::default());
    let mut builder = tar::Builder::new(encoder);
    builder.append_dir_all(".", source).unwrap();
    builder.into_inner().unwrap().finish().unwrap();
    output
}

fn profile_package(root: &Path) -> PathBuf {
    let manifest = br#"schema_version = 1
[package]
id = "aurb.core"
version = "1.0.0"
description = "Profile dependency fixture"
compatibility = { min_aletheon = "0.1.0" }
[[assets]]
kind = "agent_profile"
id = "profile.reviewer"
path = "assets/agents/reviewer.md"
"#;
    let profile = br#"---
name: aurb:reviewer
description: Aurb reviewer
tools: [aurb_search]
---
Review the candidate.
"#;
    archive(
        root,
        "profile",
        &[
            ("extension.toml", manifest),
            ("assets/agents/reviewer.md", profile),
        ],
    )
}

fn skill_package(root: &Path, version: &str) -> PathBuf {
    let manifest = format!(
        r#"schema_version = 1
[package]
id = "runtime.test"
version = "{version}"
description = "Known-good fixture"
compatibility = {{ min_aletheon = "0.1.0" }}
[[assets]]
kind = "skill"
id = "skill.known-good"
path = "assets/skills/known-good/SKILL.md"
"#
    );
    let skill = b"---\nname: known-good\ndescription: fixture\n---\n# Known good\n";
    archive(
        root,
        &format!("known-good-{version}"),
        &[
            ("extension.toml", manifest.as_bytes()),
            ("assets/skills/known-good/SKILL.md", skill),
        ],
    )
}

fn failing_runtime_package(root: &Path) -> PathBuf {
    let manifest = br#"schema_version = 1
[package]
id = "runtime.test"
version = "2.0.0"
description = "Failing runtime fixture"
compatibility = { min_aletheon = "0.1.0" }
[[assets]]
kind = "executable"
id = "runtime.failing"
path = "assets/executables/runtime.toml"
[requested_permissions]
executables = true
"#;
    let runtime = br#"schema_version = 1
id = "runtime.failing"
class = "subprocess"
protocol = "json-rpc/stdio"
command = "payload/runtime"
[isolation]
network = false
filesystem = []
cpu_time_seconds = 5
memory_bytes = 67108864
max_processes = 2
[[capabilities]]
id = "agent.failing"
kind = "agent_runtime_provider"
risk = "Sandboxed"
"#;
    let script = b"#!/bin/sh\nexit 1\n";
    let package = archive(
        root,
        "failing-runtime",
        &[
            ("extension.toml", manifest),
            ("assets/executables/runtime.toml", runtime),
            ("payload/runtime", script),
        ],
    );
    package
}

fn publisher(tools: Arc<Mutex<corpus::ToolRegistry>>) -> Arc<DaemonExtensionRuntimePublisher> {
    Arc::new(DaemonExtensionRuntimePublisher::new(
        tools,
        Arc::new(Mutex::new(corpus::HookRegistry::new(Arc::new(
            kernel::chronos::TestClock::default(),
        )))),
        SharedSkills::new(Arc::new(Vec::new())),
    ))
}

#[tokio::test]
async fn profile_requires_capabilities_from_same_candidate_snapshot() {
    let temp = TempDir::new().unwrap();
    let tools = Arc::new(Mutex::new(corpus::ToolRegistry::new()));
    let publisher = publisher(tools);
    let profiles = Arc::new(AgentProfileRegistry::default());
    publisher
        .bind_profiles(PackageProfileRuntime::new(
            profiles.clone(),
            Arc::new(NoopInference),
            Arc::new(NoopLlm),
            ExecutiveConfig::default(),
        ))
        .await
        .unwrap();
    let coordinator = ExtensionCoordinator::new(
        &temp.path().join("store"),
        ExtensionSnapshotCompiler::default(),
        publisher,
        ExtensionRuntimeView::default(),
        Arc::new(kernel::chronos::TestClock::default()),
    )
    .unwrap();
    coordinator
        .install("operator:test", &profile_package(temp.path()), false)
        .await
        .unwrap();

    let error = coordinator
        .enable("operator:test", "aurb.core", true)
        .await
        .unwrap_err();

    assert!(error
        .to_string()
        .contains("probing extension runtime candidate"));
    assert!(format!("{error:#}").contains("unknown tool 'aurb_search'"));
    assert!(!profiles.names().contains(&"aurb:reviewer".to_owned()));
}

#[tokio::test]
async fn executable_probe_failure_keeps_previous_snapshot() {
    let temp = TempDir::new().unwrap();
    let store = temp.path().join("store");
    let publisher = publisher(Arc::new(Mutex::new(corpus::ToolRegistry::new())));
    publisher
        .bind_executable_runtime(
            &temp.path().join("data"),
            &store,
            Arc::new(kernel::chronos::TestClock::default()),
            Arc::new(AgentRuntimeRegistry::default()),
        )
        .await
        .unwrap();
    let coordinator = ExtensionCoordinator::new(
        &store,
        ExtensionSnapshotCompiler::default(),
        publisher,
        ExtensionRuntimeView::default(),
        Arc::new(kernel::chronos::TestClock::default()),
    )
    .unwrap();
    coordinator
        .install("operator:test", &skill_package(temp.path(), "1.0.0"), false)
        .await
        .unwrap();
    coordinator
        .enable("operator:test", "runtime.test", true)
        .await
        .unwrap();
    let before = coordinator.view().load().await.digest.clone();
    let activation = coordinator.activation("runtime.test").unwrap();

    let error = coordinator
        .upgrade(
            "operator:test",
            &failing_runtime_package(temp.path()),
            false,
            true,
        )
        .await
        .unwrap_err();

    assert!(format!("{error:#}").contains("probing extension runtime candidate"));
    assert_eq!(coordinator.view().load().await.digest, before);
    assert_eq!(coordinator.activation("runtime.test").unwrap(), activation);
}
