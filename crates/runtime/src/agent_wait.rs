//! Runtime-owned Agent wait/timing port (RA-05).
//!
//! Waiting for a bounded Agent state change is part of the Runtime lifecycle
//! contract.  The host may provide a deterministic implementation for tests,
//! but production composition uses this dependency-free Tokio implementation;
//! Runtime does not depend on Kernel's concrete timer.

use ::contracts::AgentSnapshot;
use async_trait::async_trait;
use std::time::Duration;
use tokio::sync::watch;

#[async_trait]
pub trait AgentWaitTimer: Send + Sync {
    async fn wait_for_change(
        &self,
        receiver: &mut watch::Receiver<AgentSnapshot>,
        timeout: Duration,
    ) -> bool;
}

#[derive(Debug, Default)]
pub struct SystemAgentWaitTimer;

#[async_trait]
impl AgentWaitTimer for SystemAgentWaitTimer {
    async fn wait_for_change(
        &self,
        receiver: &mut watch::Receiver<AgentSnapshot>,
        timeout: Duration,
    ) -> bool {
        matches!(
            tokio::time::timeout(timeout, receiver.changed()).await,
            Ok(Ok(()))
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> AgentSnapshot {
        AgentSnapshot {
            handle: ::contracts::AgentHandle {
                agent_id: ::contracts::AgentId::new(),
                root_agent_id: ::contracts::AgentId::new(),
                parent_agent_id: None,
                process_id: ::contracts::ProcessId::new(),
                operation_id: ::contracts::OperationId::new(),
                runtime_id: ::contracts::RuntimeId("test".into()),
                profile_id: ::contracts::AgentProfileId("test".into()),
            },
            status: ::contracts::AgentRunStatus::Running,
            result: None,
            created_at_ms: 0,
            started_at_ms: Some(0),
            ended_at_ms: None,
            last_error: None,
        }
    }

    #[tokio::test]
    async fn timer_reports_change_before_deadline() {
        let timer = SystemAgentWaitTimer;
        let (sender, mut receiver) = watch::channel(snapshot());
        sender.send(snapshot()).unwrap();
        assert!(
            timer
                .wait_for_change(&mut receiver, Duration::from_secs(1))
                .await
        );
    }

    #[tokio::test]
    async fn timer_reports_timeout_without_change() {
        let timer = SystemAgentWaitTimer;
        let (_sender, mut receiver) = watch::channel(snapshot());
        assert!(
            !timer
                .wait_for_change(&mut receiver, Duration::from_millis(1))
                .await
        );
    }
}
