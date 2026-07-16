use std::sync::Arc;

use executive::service::agent_control::{
    AgentEventSink, AgentRuntimeEvent, MemoryRecordingAgentEventSink, NoopAgentEventSink,
};
use fabric::{AgentId, AgentTaskId, AgentRunStatus, OperationId, ProcessId};
use mnemosyne::{AgentMemoryContext, AgentMemoryVault, MemoryScope};

#[tokio::test]
async fn runtime_actions_and_results_remain_in_verified_child_scope() {
    let vault = Arc::new(AgentMemoryVault::in_memory().unwrap());
    let process = ProcessId::new();
    let agent = AgentId::new();
    let context = AgentMemoryContext::verified(
        process,
        agent,
        AgentTaskId("task:durable-request-hash".into()),
        "sha256:bounded-parent-projection",
    )
    .unwrap();
    vault.register(&context).unwrap();
    let sink = MemoryRecordingAgentEventSink::new(
        Arc::new(NoopAgentEventSink),
        vault.clone(),
        context.clone(),
    );
    sink.emit(AgentRuntimeEvent::Started {
        agent_id: agent,
        process_id: process,
        operation_id: OperationId::new(),
    })
    .await;
    sink.emit(AgentRuntimeEvent::Terminal {
        agent_id: agent,
        process_id: process,
        operation_id: OperationId::new(),
        status: AgentRunStatus::Failed,
        result: None,
    })
    .await;

    let records = vault.recall(&context).unwrap();
    assert_eq!(records.len(), 2);
    assert!(records.iter().all(|record| record.scope == context.task_scope));
    assert!(records.iter().all(|record| record.scope != MemoryScope::Global));
    assert!(sink.take_error().is_none());

    let sibling = AgentMemoryContext::verified(
        ProcessId::new(),
        AgentId::new(),
        AgentTaskId("sibling-task".into()),
        "sha256:sibling-projection",
    )
    .unwrap();
    vault.register(&sibling).unwrap();
    assert!(vault.recall(&sibling).unwrap().is_empty());
}
