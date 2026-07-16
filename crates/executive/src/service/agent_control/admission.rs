//! Root-scoped Agent topology, rollout-budget, role, and storage admission.

use std::collections::HashMap;
use std::sync::Arc;

use aletheon_kernel::admission::InMemoryBudgetController;
use async_trait::async_trait;
use cognit::config::AgentAdmissionConfig;
use fabric::{
    AgentControlError, AgentControlErrorKind, AgentId, AgentProfileId, AgentSpawnRequest,
    AttemptUsage, BudgetRequest, BudgetReservationId, BudgetScopeId, BudgetScopeKind, UsageReport,
};
use parking_lot::Mutex;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AgentStorageRequest {
    pub bytes: u64,
    pub items: u64,
}

pub struct AgentAdmissionRequest<'a> {
    pub spawn: &'a AgentSpawnRequest,
    pub depth: u16,
    pub parent_profile: Option<&'a AgentProfileId>,
    pub storage: AgentStorageRequest,
}

impl<'a> AgentAdmissionRequest<'a> {
    pub fn new(
        spawn: &'a AgentSpawnRequest,
        depth: u16,
        parent_profile: Option<&'a AgentProfileId>,
        storage: AgentStorageRequest,
    ) -> Self {
        Self {
            spawn,
            depth,
            parent_profile,
            storage,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AgentAdmissionMetrics {
    pub roots: usize,
    pub resident_agents: usize,
    pub queued_agents: usize,
    pub running_agents: usize,
    pub resident_idle_agents: usize,
    pub reserved_storage_bytes: u64,
    pub reserved_storage_items: u64,
}

#[async_trait]
pub trait AgentAdmissionLease: Send {
    async fn mark_running(&mut self) -> Result<(), AgentControlError>;
    async fn settle(&mut self, usage: &AttemptUsage) -> Result<(), AgentControlError>;
    async fn revoke(&mut self) -> Result<(), AgentControlError>;
}

#[async_trait]
pub trait AgentAdmissionPort: Send + Sync {
    async fn reserve(
        &self,
        request: AgentAdmissionRequest<'_>,
    ) -> Result<Box<dyn AgentAdmissionLease>, AgentControlError>;

    fn metrics(&self) -> AgentAdmissionMetrics;
}

#[derive(Debug, Default)]
struct RootAdmissionState {
    budget_scope: Option<BudgetScopeId>,
    resident: usize,
    queued: usize,
    running: usize,
    storage_bytes: u64,
    storage_items: u64,
    next_ticket: u64,
}

#[derive(Debug, Default)]
struct AdmissionState {
    roots: HashMap<AgentId, RootAdmissionState>,
    running_slots: usize,
}

pub struct BoundedAgentAdmission {
    config: AgentAdmissionConfig,
    budget: Arc<InMemoryBudgetController>,
    state: Arc<Mutex<AdmissionState>>,
    reservation_lock: tokio::sync::Mutex<()>,
}

impl std::fmt::Debug for BoundedAgentAdmission {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BoundedAgentAdmission")
            .field("config", &self.config)
            .field("metrics", &self.metrics())
            .finish_non_exhaustive()
    }
}

impl BoundedAgentAdmission {
    /// Compatibility constructor used by focused tests.
    pub fn new(max_concurrent: usize) -> Result<Self, AgentControlError> {
        let config = AgentAdmissionConfig {
            max_agents_per_root: max_concurrent,
            max_running_agents: max_concurrent,
            max_queued_per_root: max_concurrent,
            ..AgentAdmissionConfig::default()
        };
        Self::with_budget(config, Arc::new(InMemoryBudgetController::new()))
    }

    pub fn with_budget(
        config: AgentAdmissionConfig,
        budget: Arc<InMemoryBudgetController>,
    ) -> Result<Self, AgentControlError> {
        config
            .validate()
            .map_err(|error| AgentControlError::invalid(error.to_string()))?;
        Ok(Self {
            config,
            budget,
            state: Arc::new(Mutex::new(AdmissionState::default())),
            reservation_lock: tokio::sync::Mutex::new(()),
        })
    }

    pub fn available_permits(&self) -> usize {
        self.config
            .max_running_agents
            .saturating_sub(self.state.lock().running_slots)
    }

    fn validate_policy(
        &self,
        request: &AgentAdmissionRequest<'_>,
    ) -> Result<(), AgentControlError> {
        let budget = &request.spawn.budget;
        let requested_tokens = budget
            .max_input_tokens
            .checked_add(budget.max_output_tokens)
            .ok_or_else(|| AgentControlError::invalid("Agent token request overflow"))?;
        if request.depth > self.config.max_depth || request.depth > budget.max_depth {
            return Err(capacity("Agent tree depth is exhausted"));
        }
        if requested_tokens > self.config.max_child_tokens {
            return Err(capacity("Agent child token allowance is exhausted"));
        }
        let requested_cost = cost_micro(budget.max_cost_usd)?;
        if exceeds_optional(requested_cost, self.config.max_child_cost_micro) {
            return Err(capacity("Agent child cost allowance is exhausted"));
        }
        if request.storage.bytes > self.config.max_storage_bytes
            || request.storage.items > self.config.max_storage_items
        {
            return Err(capacity("Agent storage request exceeds policy"));
        }
        if request
            .parent_profile
            .is_some_and(|profile| is_non_delegating_role(&profile.0))
        {
            return Err(AgentControlError {
                kind: AgentControlErrorKind::Forbidden,
                message: "internal memory/consolidation Agent roles cannot delegate".into(),
            });
        }
        Ok(())
    }
}

#[async_trait]
impl AgentAdmissionPort for BoundedAgentAdmission {
    async fn reserve(
        &self,
        request: AgentAdmissionRequest<'_>,
    ) -> Result<Box<dyn AgentAdmissionLease>, AgentControlError> {
        let _reservation_guard = self.reservation_lock.lock().await;
        self.validate_policy(&request)?;
        let root = request.spawn.root_agent_id;
        let (root_scope, ticket) = {
            let mut state = self.state.lock();
            if state.running_slots >= self.config.max_running_agents {
                return Err(capacity("Agent global running capacity is exhausted"));
            }
            let root_state = state.roots.entry(root).or_default();
            if root_state.resident >= self.config.max_agents_per_root {
                return Err(capacity("Agent root tree capacity is exhausted"));
            }
            if root_state.queued >= self.config.max_queued_per_root {
                return Err(capacity("Agent root queue capacity is exhausted"));
            }
            if root_state
                .storage_bytes
                .checked_add(request.storage.bytes)
                .is_none_or(|value| value > self.config.max_storage_bytes)
                || root_state
                    .storage_items
                    .checked_add(request.storage.items)
                    .is_none_or(|value| value > self.config.max_storage_items)
            {
                return Err(capacity("Agent root storage capacity is exhausted"));
            }
            root_state.resident += 1;
            root_state.queued += 1;
            root_state.storage_bytes += request.storage.bytes;
            root_state.storage_items += request.storage.items;
            let ticket = root_state.next_ticket;
            root_state.next_ticket = root_state
                .next_ticket
                .saturating_add(self.config.sibling_fairness_quantum as u64);
            let budget_scope = root_state.budget_scope;
            state.running_slots += 1;
            (budget_scope, ticket)
        };

        let root_scope = match root_scope {
            Some(scope) => scope,
            None => {
                let scope = self
                    .budget
                    .create_root(
                        format!("agent-root:{}", root.0),
                        BudgetRequest {
                            max_tokens: Some(self.config.root_max_tokens),
                            max_cost_micro: self.config.root_max_cost_micro,
                        },
                    )
                    .await;
                let mut state = self.state.lock();
                let root_state = state.roots.get_mut(&root).expect("root reservation exists");
                *root_state.budget_scope.get_or_insert(scope)
            }
        };

        let requested_tokens = request
            .spawn
            .budget
            .max_input_tokens
            .checked_add(request.spawn.budget.max_output_tokens)
            .expect("validated token sum");
        let reservation = self
            .budget
            .reserve_child(
                root_scope,
                BudgetScopeKind::Process,
                format!("agent-ticket:{}:{ticket}", root.0),
                BudgetRequest {
                    max_tokens: Some(requested_tokens),
                    max_cost_micro: cost_micro(request.spawn.budget.max_cost_usd)?,
                },
            )
            .await;
        let reservation = match reservation {
            Ok(reservation) => reservation,
            Err(_) => {
                release_topology(&self.state, root, request.storage, LeasePhase::Queued);
                return Err(capacity("Agent root rollout budget is exhausted"));
            }
        };
        Ok(Box::new(BoundedAdmissionLease {
            state: self.state.clone(),
            budget: self.budget.clone(),
            root,
            storage: request.storage,
            reservation: Some(reservation.reservation_id),
            phase: LeasePhase::Queued,
        }))
    }

    fn metrics(&self) -> AgentAdmissionMetrics {
        let state = self.state.lock();
        AgentAdmissionMetrics {
            roots: state.roots.len(),
            resident_agents: state.roots.values().map(|root| root.resident).sum(),
            queued_agents: state.roots.values().map(|root| root.queued).sum(),
            running_agents: state.roots.values().map(|root| root.running).sum(),
            resident_idle_agents: state
                .roots
                .values()
                .map(|root| root.resident.saturating_sub(root.queued + root.running))
                .sum(),
            reserved_storage_bytes: state.roots.values().map(|root| root.storage_bytes).sum(),
            reserved_storage_items: state.roots.values().map(|root| root.storage_items).sum(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LeasePhase {
    Queued,
    Running,
    Closed,
}

struct BoundedAdmissionLease {
    state: Arc<Mutex<AdmissionState>>,
    budget: Arc<InMemoryBudgetController>,
    root: AgentId,
    storage: AgentStorageRequest,
    reservation: Option<BudgetReservationId>,
    phase: LeasePhase,
}

#[async_trait]
impl AgentAdmissionLease for BoundedAdmissionLease {
    async fn mark_running(&mut self) -> Result<(), AgentControlError> {
        if self.phase != LeasePhase::Queued {
            return Err(conflict("Agent admission lease is not queued"));
        }
        let mut state = self.state.lock();
        let root = state.roots.get_mut(&self.root).expect("root lease exists");
        root.queued = root.queued.saturating_sub(1);
        root.running += 1;
        self.phase = LeasePhase::Running;
        Ok(())
    }

    async fn settle(&mut self, usage: &AttemptUsage) -> Result<(), AgentControlError> {
        if self.phase == LeasePhase::Closed {
            return Err(conflict("Agent admission lease is already closed"));
        }
        let reservation = self
            .reservation
            .as_ref()
            .copied()
            .ok_or_else(|| conflict("Agent admission lease has no live budget reservation"))?;
        self.budget
            .settle_reservation(
                reservation,
                &UsageReport {
                    tokens_used: usage.input_tokens.saturating_add(usage.output_tokens),
                    cost_micro: cost_micro(usage.cost_usd)?.unwrap_or(0),
                    wall_time_ms: usage.elapsed_ms,
                    ..UsageReport::default()
                },
            )
            .await
            .map_err(|error| conflict(format!("Agent budget settlement failed: {error}")))?;
        self.reservation = None;
        release_topology(&self.state, self.root, self.storage, self.phase);
        self.phase = LeasePhase::Closed;
        Ok(())
    }

    async fn revoke(&mut self) -> Result<(), AgentControlError> {
        if self.phase == LeasePhase::Closed {
            return Ok(());
        }
        if let Some(reservation) = self.reservation.take() {
            self.budget
                .revoke_reservation(reservation)
                .await
                .map_err(|error| conflict(format!("Agent budget revoke failed: {error}")))?;
        }
        release_topology(&self.state, self.root, self.storage, self.phase);
        self.phase = LeasePhase::Closed;
        Ok(())
    }
}

impl Drop for BoundedAdmissionLease {
    fn drop(&mut self) {
        if self.phase == LeasePhase::Closed {
            return;
        }
        release_topology(&self.state, self.root, self.storage, self.phase);
        self.phase = LeasePhase::Closed;
        if let Some(reservation) = self.reservation.take() {
            let budget = self.budget.clone();
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(async move {
                    let _ = budget.revoke_reservation(reservation).await;
                });
            }
        }
    }
}

fn release_topology(
    state: &Arc<Mutex<AdmissionState>>,
    root_id: AgentId,
    storage: AgentStorageRequest,
    phase: LeasePhase,
) {
    let mut state = state.lock();
    if let Some(root) = state.roots.get_mut(&root_id) {
        root.resident = root.resident.saturating_sub(1);
        match phase {
            LeasePhase::Queued => root.queued = root.queued.saturating_sub(1),
            LeasePhase::Running => root.running = root.running.saturating_sub(1),
            LeasePhase::Closed => {}
        }
        root.storage_bytes = root.storage_bytes.saturating_sub(storage.bytes);
        root.storage_items = root.storage_items.saturating_sub(storage.items);
    }
    state.running_slots = state.running_slots.saturating_sub(1);
}

fn cost_micro(cost_usd: Option<f64>) -> Result<Option<u64>, AgentControlError> {
    cost_usd
        .map(|cost| {
            if !cost.is_finite() || cost < 0.0 || cost > u64::MAX as f64 / 1_000_000.0 {
                return Err(AgentControlError::invalid("Agent cost budget is invalid"));
            }
            Ok((cost * 1_000_000.0).ceil() as u64)
        })
        .transpose()
}

fn exceeds_optional(requested: Option<u64>, limit: Option<u64>) -> bool {
    match (requested, limit) {
        (Some(_), None) => false,
        (Some(requested), Some(limit)) => requested > limit,
        (None, Some(_)) => true,
        (None, None) => false,
    }
}

fn is_non_delegating_role(profile: &str) -> bool {
    let profile = profile.to_ascii_lowercase();
    ["memory", "mnemosyne", "consolidat", "internal-recall"]
        .iter()
        .any(|role| profile.contains(role))
}

fn capacity(message: impl Into<String>) -> AgentControlError {
    AgentControlError {
        kind: AgentControlErrorKind::Capacity,
        message: message.into(),
    }
}

fn conflict(message: impl Into<String>) -> AgentControlError {
    AgentControlError {
        kind: AgentControlErrorKind::Conflict,
        message: message.into(),
    }
}
