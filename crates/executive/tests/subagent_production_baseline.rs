use std::collections::HashMap;
use std::sync::Arc;

use aletheon_kernel::chronos::TestClock;
use aletheon_kernel::operation::OperationTable;
use aletheon_kernel::process::ProcessTable;
use aletheon_kernel::supervision::RestartPolicy;
use corpus::tools::tools::agent_tool::{AgentDefinition, AgentTool, ExecuteSubAgentFn};
use fabric::{Clock, ProcessSnapshot, SubAgentState, Tool, ToolContext};
use tokio::sync::Mutex;

use executive::core::SubAgentSpawner;

fn clock() -> Arc<dyn Clock> {
    Arc::new(TestClock::default())
}

fn definition() -> AgentDefinition {
    AgentDefinition {
        name: "reviewer".into(),
        description: "bounded reviewer".into(),
        tools: vec!["file_read".into(), "grep".into()],
        model: Some("profile-model".into()),
        max_iterations: 7,
        system_prompt: "Review evidence only.".into(),
    }
}

fn context(root: &std::path::Path) -> ToolContext {
    ToolContext {
        working_dir: root.to_path_buf(),
        session_id: "root-session".into(),
        clock: clock(),
    }
}

#[tokio::test]
async fn successful_agent_tool_vertical_slice_records_kernel_and_mailbox_evidence() {
    let process_table = Arc::new(ProcessTable::new(clock()));
    let operation_table = Arc::new(OperationTable::new(clock()));
    let spawner = Arc::new(Mutex::new(SubAgentSpawner::with_tables(
        process_table,
        operation_table,
        clock(),
    )));
    let observed = Arc::new(Mutex::new(None::<(Vec<String>, ProcessSnapshot, bool)>));
    let closure_spawner = spawner.clone();
    let closure_observed = observed.clone();
    let execute: ExecuteSubAgentFn = Arc::new(move |_, prompt, allowed_tools| {
        let spawner = closure_spawner.clone();
        let observed = closure_observed.clone();
        Box::pin(async move {
            let mut spawner = spawner.lock().await;
            let handle = spawner
                .spawn_tracked(prompt, "root-turn".into(), RestartPolicy::Never)
                .await?;
            spawner
                .transition(&handle.id, SubAgentState::Running)
                .await?;
            let snapshot = spawner.snapshot(&handle.id).await?.unwrap();
            let mailbox_exists = spawner.mailbox_target(&handle.id).is_some();
            *observed.lock().await = Some((allowed_tools, snapshot, mailbox_exists));
            spawner
                .transition(&handle.id, SubAgentState::Completed)
                .await?;
            spawner.destroy(&handle.id).await?;
            Ok("review complete".into())
        })
    });
    let mut agents = HashMap::new();
    agents.insert("reviewer".into(), definition());
    let tool = AgentTool::new(agents, execute);
    let temp = tempfile::tempdir().unwrap();

    let result = tool
        .execute(
            serde_json::json!({"agent_type":"reviewer","prompt":"inspect the plan"}),
            &context(temp.path()),
        )
        .await;

    assert!(!result.is_error);
    assert_eq!(result.content, "review complete");
    let (tools, snapshot, mailbox) = observed.lock().await.clone().unwrap();
    assert_eq!(tools, vec!["file_read", "grep"]);
    assert!(snapshot.active_operation.is_some());
    assert!(mailbox);
    assert!(spawner.lock().await.list().is_empty());
}

#[tokio::test]
async fn error_and_unknown_profile_map_without_implicit_execution() {
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let closure_calls = calls.clone();
    let execute: ExecuteSubAgentFn = Arc::new(move |_, _, _| {
        closure_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Box::pin(async { anyhow::bail!("provider unavailable") })
    });
    let mut agents = HashMap::new();
    agents.insert("reviewer".into(), definition());
    let tool = AgentTool::new(agents, execute);
    let temp = tempfile::tempdir().unwrap();

    let failed = tool
        .execute(
            serde_json::json!({"agent_type":"reviewer","prompt":"inspect"}),
            &context(temp.path()),
        )
        .await;
    assert!(failed.is_error);
    assert!(failed.content.contains("provider unavailable"));

    let unknown = tool
        .execute(
            serde_json::json!({"agent_type":"missing","prompt":"inspect"}),
            &context(temp.path()),
        )
        .await;
    assert!(unknown.is_error);
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[tokio::test]
#[ignore = "known G04 gap: inline AgentTool loop does not connect cancellation to provider and tool work"]
async fn cancellation_interrupts_live_child_runtime() {
    // G04 replaces this assertion body with the same fixture routed through
    // NativeCognitRuntime, then removes the ignore marker.
    panic!("G04 target: cancellation must interrupt the live child runtime");
}

#[tokio::test]
#[ignore = "known G04 gap: ExecuteSubAgentFn drops AgentDefinition model and max_iterations"]
async fn profile_model_and_iteration_limit_reach_child_session() {
    // The current callback signature cannot observe these fields. G04 keeps
    // this acceptance name and asserts them on the child CognitiveSession.
    panic!("G04 target: profile model and max_iterations must reach the child session");
}
