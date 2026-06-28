//! AgentProcess — a process-like agent entity.
//!
//! Has PID, state machine, energy budget, lifecycle management.
//! Can spawn child processes. Consumes LlmPulse energy to think and act.

use std::sync::Arc;
use std::time::Duration;
use anyhow::Result;
use tokio::sync::RwLock;
use aletheon_abi::agent::Pid;
use aletheon_abi::{EventBus, EventType, Priority};
use aletheon_abi::evolution::{
    AgentStartedPayload, AgentStoppedPayload, AgentSpawnedPayload,
    CognitivePulseEvent,
};
use aletheon_comm::ConcreteEvent;
use super::budget::TokenBudget;
use crate::r#impl::engine::cognitive_loop::{Engine, TurnResult};

/// Agent lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentState {
    Idle,
    Thinking,
    Acting,
    Reflecting,
    Sleeping,
    Terminated,
}

/// Configuration for an AgentProcess.
#[derive(Debug, Clone)]
pub struct AgentProcessConfig {
    pub max_tokens_per_pulse: u32,
    pub max_children: usize,
    pub idle_timeout: Duration,
    pub can_spawn: bool,
}

impl Default for AgentProcessConfig {
    fn default() -> Self {
        Self {
            max_tokens_per_pulse: 50_000,
            max_children: 4,
            idle_timeout: Duration::from_secs(300),
            can_spawn: true,
        }
    }
}

/// A process-like agent entity.
pub struct AgentProcess {
    pub pid: Pid,
    state: AgentState,
    parent: Option<Pid>,
    children: RwLock<Vec<Pid>>,
    energy: TokenBudget,
    engine: Option<Engine>,
    task: String,
    bus: Arc<dyn EventBus>,
    config: AgentProcessConfig,
}

impl AgentProcess {
    pub fn new(
        parent: Option<Pid>,
        task: String,
        bus: Arc<dyn EventBus>,
        config: AgentProcessConfig,
    ) -> Self {
        Self {
            pid: Pid::new(),
            state: AgentState::Idle,
            parent,
            children: RwLock::new(Vec::new()),
            energy: TokenBudget::new(config.max_tokens_per_pulse),
            engine: None,
            task,
            bus,
            config,
        }
    }

    /// Start the agent: publish AgentStarted event.
    pub async fn start(&mut self) -> Result<()> {
        self.state = AgentState::Idle;

        self.bus.publish(Box::new(ConcreteEvent::new(
            EventType::AgentStarted,
            Priority::Normal,
            format!("agent:{}", self.pid),
            Box::new(AgentStartedPayload {
                pid: self.pid.as_u64(),
                task: self.task.clone(),
            }),
        ))).await?;

        tracing::info!("Agent {} started: {}", self.pid, self.task);
        Ok(())
    }

    /// Handle a cognitive pulse — consume energy to think.
    pub async fn on_pulse(&mut self, pulse: &CognitivePulseEvent) -> Result<()> {
        if self.state == AgentState::Idle
            || self.state == AgentState::Sleeping
            || self.state == AgentState::Terminated
        {
            return Ok(());
        }

        let budget = self.energy.claim(pulse.available_tokens);
        if budget == 0 {
            return Ok(());
        }

        if let Some(engine) = &mut self.engine {
            self.state = AgentState::Thinking;

            let result = engine.run_turn_with_budget(&self.task, budget).await;
            self.state = match result {
                TurnResult::Complete(_) => AgentState::Idle,
                TurnResult::NeedTool { .. } => AgentState::Acting,
                TurnResult::NeedReflection => AgentState::Reflecting,
                TurnResult::Error(ref e) => {
                    tracing::warn!("Agent {} turn error: {}", self.pid, e);
                    AgentState::Idle
                }
            };
        }

        Ok(())
    }

    /// Spawn a child agent.
    pub async fn spawn_child(&self, child_task: String) -> Result<Pid> {
        if !self.config.can_spawn {
            anyhow::bail!("Agent {} cannot spawn children", self.pid);
        }

        let children = self.children.read().await;
        if children.len() >= self.config.max_children {
            anyhow::bail!("Agent {} max children ({}) reached", self.pid, self.config.max_children);
        }
        drop(children);

        let child_config = AgentProcessConfig {
            max_tokens_per_pulse: self.config.max_tokens_per_pulse / 2,
            max_children: 0, // leaf agent
            can_spawn: false,
            ..self.config.clone()
        };

        let mut child = AgentProcess::new(
            Some(self.pid),
            child_task,
            self.bus.clone(),
            child_config,
        );
        child.start().await?;
        let child_pid = child.pid;

        self.children.write().await.push(child_pid);

        self.bus.publish(Box::new(ConcreteEvent::new(
            EventType::AgentSpawned,
            Priority::Normal,
            format!("agent:{}", self.pid),
            Box::new(AgentSpawnedPayload {
                parent: self.pid.as_u64(),
                child: child_pid.as_u64(),
            }),
        ))).await?;

        Ok(child_pid)
    }

    /// Terminate the agent.
    pub async fn terminate(&mut self) -> Result<()> {
        self.state = AgentState::Terminated;

        self.bus.publish(Box::new(ConcreteEvent::new(
            EventType::AgentStopped,
            Priority::Normal,
            format!("agent:{}", self.pid),
            Box::new(AgentStoppedPayload {
                pid: self.pid.as_u64(),
            }),
        ))).await?;

        tracing::info!("Agent {} terminated", self.pid);
        Ok(())
    }

    // Accessors
    pub fn pid(&self) -> Pid { self.pid }
    pub fn state(&self) -> AgentState { self.state }
    pub fn task(&self) -> &str { &self.task }
    pub fn parent(&self) -> Option<Pid> { self.parent }
    pub fn energy(&self) -> &TokenBudget { &self.energy }

    /// Attach an Engine to this agent.
    pub fn set_engine(&mut self, engine: Engine) {
        self.engine = Some(engine);
    }
}
