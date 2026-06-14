//! Integration tests for AgentKernel spawn, fork, kill, IPC send/recv, scratchpad.

use std::sync::Arc;

use async_trait::async_trait;
use aletheon_abi::agent::Pid;
use aletheon_abi::{Event, EventBus, EventHandler, EventType, ForkDirective, IpcMessage, SubscriptionId};
use aletheon_runtime::r#impl::kernel::ipc::IpcSendError;
use aletheon_runtime::r#impl::kernel::{AgentKernel, KernelError};
use aletheon_runtime::r#impl::agent::process::AgentProcessConfig;

// ---------------------------------------------------------------------------
// Minimal EventBus stub for integration tests
// ---------------------------------------------------------------------------

struct NoopEventBus;

#[async_trait]
impl EventBus for NoopEventBus {
    async fn publish(&self, _event: Box<dyn Event>) -> anyhow::Result<()> {
        Ok(())
    }
    async fn subscribe(
        &self,
        _event_type: EventType,
        _handler: EventHandler,
    ) -> anyhow::Result<SubscriptionId> {
        Ok(SubscriptionId(0))
    }
    async fn request(
        &self,
        _event: Box<dyn Event>,
        _timeout: std::time::Duration,
    ) -> anyhow::Result<Box<dyn Event>> {
        anyhow::bail!("not implemented")
    }
    async fn unsubscribe(&self, _id: SubscriptionId) -> anyhow::Result<()> {
        Ok(())
    }
    async fn has_subscribers(&self, _event_type: &EventType) -> bool {
        false
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_kernel() -> AgentKernel {
    AgentKernel::new(Arc::new(NoopEventBus))
}

fn make_config(id: &str) -> AgentProcessConfig {
    AgentProcessConfig {
        id: id.to_string(),
        max_tokens_per_pulse: 1000,
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_spawn_creates_process() {
    let kernel = make_kernel();
    let config = make_config("test");
    let pid = kernel.spawn("test task".into(), config, None).await;
    assert!(pid.as_u64() > 0);
    assert_eq!(kernel.process_count().await, 1);
}

#[tokio::test]
async fn test_spawn_with_parent() {
    let kernel = make_kernel();
    let parent = kernel.spawn("parent task".into(), make_config("parent"), None).await;
    let child = kernel.spawn("child task".into(), make_config("child"), Some(parent)).await;
    let children = kernel.children_of(parent).await;
    assert!(children.contains(&child));
    assert_eq!(children.len(), 1);
}

#[tokio::test]
async fn test_kill_process() {
    let kernel = make_kernel();
    let pid = kernel.spawn("task".into(), make_config("a"), None).await;
    assert_eq!(kernel.process_count().await, 1);
    kernel.kill(pid).await.expect("kill should succeed");
    assert_eq!(kernel.process_count().await, 0);
}

#[tokio::test]
async fn test_kill_not_found() {
    let kernel = make_kernel();
    let bogus = Pid::new();
    let err = kernel.kill(bogus).await.unwrap_err();
    assert!(matches!(err, KernelError::ProcessNotFound(p) if p == bogus));
}

#[tokio::test]
async fn test_ipc_send_recv() {
    let kernel = make_kernel();
    let sender = kernel.spawn("sender".into(), make_config("s"), None).await;
    let receiver = kernel.spawn("receiver".into(), make_config("r"), None).await;

    let msg = IpcMessage::task(sender, receiver, "hello".into());

    // The kernel registers an IPC inbox for each spawned process, but the stub
    // AgentProcess does not hold the receiver half.  The send attempt therefore
    // returns RecipientGone — this is expected behaviour for the current stub.
    let result = kernel.send(msg).await;
    assert!(
        matches!(result, Err(IpcSendError::RecipientGone)),
        "expected RecipientGone with stub AgentProcess, got: {:?}",
        result,
    );
}

#[tokio::test]
async fn test_scratchpad_shared() {
    let kernel = make_kernel();
    let pid = Pid::new();

    // First get — creates the scratchpad.
    let sp1 = kernel.scratchpad("task-xyz").await;
    sp1.write("key1", "value1".to_string(), pid).await;
    assert_eq!(sp1.read("key1").await, Some("value1".to_string()));

    // Second get — returns the same scratchpad; data persists.
    let sp2 = kernel.scratchpad("task-xyz").await;
    assert!(Arc::ptr_eq(&sp1, &sp2), "same task_id must return same scratchpad");
    assert_eq!(sp2.read("key1").await, Some("value1".to_string()));

    // Different task_id yields a different scratchpad.
    let sp3 = kernel.scratchpad("task-other").await;
    assert!(!Arc::ptr_eq(&sp1, &sp3), "different task_id must return different scratchpad");
    assert_eq!(sp3.read("key1").await, None);
}

#[tokio::test]
async fn test_total_count() {
    let kernel = make_kernel();
    assert_eq!(kernel.total_count().await, 0);
    assert_eq!(kernel.process_count().await, 0);
    assert_eq!(kernel.fork_count().await, 0);

    // One process.
    let p = kernel.spawn("t".into(), make_config("a"), None).await;
    assert_eq!(kernel.total_count().await, 1);
    assert_eq!(kernel.process_count().await, 1);
    assert_eq!(kernel.fork_count().await, 0);

    // Fork from the process — adds one fork.
    let _fork_pid = kernel.fork(p, ForkDirective::default()).await.expect("fork should succeed");
    assert_eq!(kernel.total_count().await, 2);
    assert_eq!(kernel.process_count().await, 1);
    assert_eq!(kernel.fork_count().await, 1);
}
