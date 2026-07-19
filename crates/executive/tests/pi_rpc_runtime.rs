use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use async_trait::async_trait;
use executive::r#impl::runtime::PiRpcRuntime;
use executive::service::agent_control::{
    AgentContextProjection, AgentEventSink, AgentRuntimeEvent, AgentRuntimeInbox,
    AgentRuntimeInput, AgentRuntimeLauncher,
};
use fabric::sandbox::{
    IsolationLevel, SandboxBackend, SandboxCapabilities, SandboxCommand, SandboxConfig,
    SandboxResult,
};
use fabric::{
    AgentBudget, AgentContextFork, AgentHandle, AgentId, AgentMessageKind, AgentMessagePayload,
    AgentProfileId, AgentSpawnRequest, AgoraSpaceId, OperationId, ProcessId, RuntimeId,
    WorkspacePolicy, AGENT_MESSAGE_SCHEMA_V1,
};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

struct FixtureSandbox {
    script: PathBuf,
}

#[async_trait]
impl SandboxBackend for FixtureSandbox {
    fn name(&self) -> &str {
        "fixture-namespace"
    }
    fn isolation_level(&self) -> IsolationLevel {
        IsolationLevel::Namespace
    }
    fn is_available(&self) -> bool {
        true
    }
    fn capabilities(&self) -> SandboxCapabilities {
        SandboxCapabilities {
            filesystem_isolation: true,
            network_isolation: true,
            resource_limits: true,
            seccomp_filter: true,
            limitations: vec![],
        }
    }
    fn wrap_argv(
        &self,
        _program: &Path,
        args: &[String],
        _config: &SandboxConfig,
    ) -> Result<SandboxCommand> {
        assert!(args.windows(2).any(|pair| pair == ["--mode", "rpc"]));
        Ok(SandboxCommand {
            program: "/bin/sh".into(),
            args: vec![self.script.to_string_lossy().into_owned()],
            environment: BTreeMap::new(),
        })
    }
    async fn execute(
        &self,
        _cmd: &str,
        _config: &SandboxConfig,
        _timeout: std::time::Duration,
    ) -> Result<SandboxResult> {
        unreachable!()
    }
}

#[derive(Default)]
struct Events(Mutex<Vec<AgentRuntimeEvent>>);

#[async_trait]
impl AgentEventSink for Events {
    async fn emit(&self, event: AgentRuntimeEvent) {
        self.0.lock().unwrap().push(event);
    }
}

fn fixed_args() -> Vec<String> {
    [
        "--mode",
        "json",
        "--no-session",
        "--no-context-files",
        "--no-extensions",
        "--no-skills",
        "--no-prompt-templates",
        "--no-themes",
        "--no-approve",
        "--offline",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn input_with_inbox(
    policy: WorkspacePolicy,
    label: &str,
) -> (
    tokio::sync::mpsc::Sender<AgentMessagePayload>,
    AgentRuntimeInput,
) {
    let (sender, inbox) = AgentRuntimeInbox::bounded_channel(4).unwrap();
    let agent = AgentId::new();
    let process = ProcessId::new();
    let request = AgentSpawnRequest {
        root_agent_id: agent,
        parent_agent_id: None,
        parent_process_id: None,
        profile_id: AgentProfileId("pi".into()),
        runtime_id: RuntimeId("pi-rpc".into()),
        trusted_workspace: Some(policy.clone()),
        task: format!("start-{label}"),
        context: AgentContextFork::None,
        broadcast_refs: vec![],
        allowed_tools: vec![],
        background_decls: vec![],
        budget: AgentBudget {
            max_input_tokens: 100,
            max_output_tokens: 100,
            max_tool_calls: 10,
            max_elapsed_ms: 5_000,
            max_cost_usd: None,
            max_depth: 1,
        },
    };
    let input = AgentRuntimeInput {
        workspace: Some(policy),
        context: AgentContextProjection::from_fork(&request.context).unwrap(),
        memory_context: mnemosyne::AgentMemoryContext::verified(
            process,
            agent,
            fabric::AgentTaskId(format!("pi-test-{label}")),
            "sha256:pi-test",
        )
        .unwrap(),
        request,
        handle: AgentHandle {
            agent_id: agent,
            root_agent_id: agent,
            parent_agent_id: None,
            process_id: process,
            operation_id: OperationId::new(),
            runtime_id: RuntimeId("pi-rpc".into()),
            profile_id: AgentProfileId("pi".into()),
        },
        workspace_id: AgoraSpaceId(format!("pi-private-{label}")),
        root_workspace_id: AgoraSpaceId(format!("pi-root-{label}")),
        root_process_id: process,
        inbox,
        cancellation: CancellationToken::new(),
        background_cancellations: std::collections::HashMap::new(),
        background_registrations: std::collections::HashMap::new(),
        background_notification_targets: std::collections::HashMap::new(),
    };
    (sender, input)
}

#[tokio::test]
async fn resident_runtime_maps_mailbox_commands_correlates_state_and_settles() {
    let temp = TempDir::new().unwrap();
    let workspace = temp.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let script = temp.path().join("pi-rpc-fixture.sh");
    std::fs::write(&script, r#"
count=0
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  type=$(printf '%s' "$line" | sed -n 's/.*"type":"\([^"]*\)".*/\1/p')
  printf '{"type":"response","command":"%s","id":"%s","success":true' "$type" "$id"
  if [ "$type" = get_state ]; then printf ',"data":{"isStreaming":false}}\n'; exit 0; else printf '}\n'; fi
  if [ "$type" = prompt ]; then printf '{"type":"agent_start"}\n'; fi
  if [ "$type" = steer ]; then printf '{"type":"queue_update","steering":[]}\n'; fi
  if [ "$type" = follow_up ]; then
    printf '{"type":"tool_execution_end","toolCallId":"t1","toolName":"bash","result":{"content":"ok"},"isError":false}\n'
    printf '{"type":"message_end","message":{"role":"assistant","content":"done","usage":{"inputTokens":5,"outputTokens":3}}}\n'
    printf '{"type":"agent_settled"}\n'
  fi
done
"#).unwrap();

    let policy = WorkspacePolicy::from_resolved_roots(workspace.clone(), vec![]).unwrap();
    let executable_sha256 = format!("{:x}", Sha256::digest(std::fs::read(&script).unwrap()));
    let config = cognit::config::PiRuntimeConfig {
        enabled: true,
        executable: script.clone(),
        trusted_executable_dir: None,
        fixed_args: fixed_args(),
        package_version: "0.80.10".into(),
        executable_sha256,
        json_protocol_version: 3,
        worktree_base: workspace.clone(),
        timeout_ms: 5_000,
        max_output_bytes: 64 * 1024,
        allowed_paths: vec![PathBuf::from(".")],
        forbidden_paths: vec![],
        require_namespace_isolation: true,
        network_enabled: false,
    };
    let runtime = Arc::new(
        PiRpcRuntime::prepare(
            &config,
            Arc::new(FixtureSandbox { script }),
            Arc::new(aletheon_kernel::chronos::SystemClock::new()),
            BTreeMap::new(),
        )
        .unwrap()
        .unwrap(),
    );
    let mut tasks = Vec::new();
    for label in ["one", "two"] {
        let (sender, input) = input_with_inbox(policy.clone(), label);
        for (content, start_turn) in [("steer now", false), ("then finish", true)] {
            sender
                .send(AgentMessagePayload {
                    schema_version: AGENT_MESSAGE_SCHEMA_V1,
                    kind: AgentMessageKind::Input,
                    content: content.into(),
                    start_turn,
                    correlation_id: None,
                    deadline_mono_ms: None,
                })
                .await
                .unwrap();
        }
        let runtime = runtime.clone();
        let events = Arc::new(Events::default());
        tasks.push(tokio::spawn(async move {
            (runtime.launch(input, events.clone()).await, events)
        }));
    }
    for task in tasks {
        let (result, events) = task.await.unwrap();
        let result = result.unwrap();
        assert_eq!(result.output, "done");
        assert_eq!(result.usage.input_tokens, 5);
        assert_eq!(result.evidence.len(), 1);
        assert!(events.0.lock().unwrap().iter().any(|event| matches!(
            event,
            AgentRuntimeEvent::Terminal {
                status: fabric::AgentRunStatus::Succeeded,
                ..
            }
        )));
    }
}

#[test]
fn trusted_workspace_is_not_deserializable_or_serialized() {
    let value = serde_json::json!({
        "root_agent_id": AgentId::new(), "parent_agent_id": null, "parent_process_id": null,
        "profile_id":"p", "runtime_id":"pi-rpc", "trusted_workspace":{"cwd":"/tmp","writable_roots":["/tmp"]},
        "task":"x", "context":{"mode":"none"}, "broadcast_refs":[], "allowed_tools":[],
        "budget":{"max_input_tokens":1,"max_output_tokens":1,"max_tool_calls":1,"max_elapsed_ms":1,"max_cost_usd":null,"max_depth":1}
    });
    let request: AgentSpawnRequest = serde_json::from_value(value).unwrap();
    assert!(request.trusted_workspace.is_none());
    assert!(serde_json::to_value(request)
        .unwrap()
        .get("trusted_workspace")
        .is_none());
}

#[tokio::test]
async fn cancellation_kills_the_resident_process_group_and_descendant() {
    let temp = TempDir::new().unwrap();
    let workspace = temp.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let script = temp.path().join("pi-rpc-descendant.sh");
    std::fs::write(
        &script,
        r#"
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  type=$(printf '%s' "$line" | sed -n 's/.*"type":"\([^"]*\)".*/\1/p')
  printf '{"type":"response","command":"%s","id":"%s","success":true}\n' "$type" "$id"
  if [ "$type" = prompt ]; then
    printf '{"type":"agent_start"}\n'
    sleep 60 &
    printf '%s\n' "$!" > descendant.pid
  fi
done
"#,
    )
    .unwrap();
    let executable_sha256 = format!("{:x}", Sha256::digest(std::fs::read(&script).unwrap()));
    let config = cognit::config::PiRuntimeConfig {
        enabled: true,
        executable: script.clone(),
        trusted_executable_dir: None,
        fixed_args: fixed_args(),
        package_version: "0.80.10".into(),
        executable_sha256,
        json_protocol_version: 3,
        worktree_base: workspace.clone(),
        timeout_ms: 5_000,
        max_output_bytes: 64 * 1024,
        allowed_paths: vec![PathBuf::from(".")],
        forbidden_paths: vec![],
        require_namespace_isolation: true,
        network_enabled: false,
    };
    let runtime = PiRpcRuntime::prepare(
        &config,
        Arc::new(FixtureSandbox { script }),
        Arc::new(aletheon_kernel::chronos::SystemClock::new()),
        BTreeMap::new(),
    )
    .unwrap()
    .unwrap();
    let policy = WorkspacePolicy::from_resolved_roots(workspace.clone(), vec![]).unwrap();
    let (_sender, input) = input_with_inbox(policy, "cancel");
    let cancellation = input.cancellation.clone();
    let task =
        tokio::spawn(async move { runtime.launch(input, Arc::new(Events::default())).await });
    let pid_path = workspace.join("descendant.pid");
    let descendant = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if let Ok(text) = std::fs::read_to_string(&pid_path) {
                if let Ok(pid) = text.trim().parse::<i32>() {
                    break pid;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("fixture descendant pid");
    cancellation.cancel();
    let error = task.await.unwrap().unwrap_err();
    assert_eq!(error.kind, fabric::AgentControlErrorKind::Terminal);
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let alive = unsafe { libc::kill(descendant, 0) } == 0;
            if !alive {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("descendant process group was not reaped");
}

#[test]
fn rpc_environment_uses_a_reviewed_path_not_the_parent_path() {
    let environment = executive::r#impl::runtime::pi_rpc_environment_from_process();
    assert_eq!(
        environment.get("PATH").map(String::as_str),
        Some("/usr/local/bin:/usr/bin:/bin")
    );
}
