//! Hierarchical in-memory monetary budget controller.
//!
//! Budget ownership is one tree per rollout:
//! rollout -> process -> operation -> capability. Allocating a child holds
//! capacity in its direct parent; settling or revoking closes the child and
//! returns unused capacity exactly once. A compatibility adapter preserves the
//! principal-based admission API by creating and closing a complete temporary
//! hierarchy for each permit.

use async_trait::async_trait;
use fabric::{
    AdmissionError, BudgetController, BudgetRequest, BudgetReservationId, BudgetReservationReceipt,
    BudgetScope, BudgetScopeId, BudgetScopeKind, BudgetTransferReceipt, OperationId, PermitId,
    UsageReport, BUDGET_SCOPE_SCHEMA_VERSION,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::sync::Mutex;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct Amount {
    tokens: Option<u64>,
    cost_micro: Option<u64>,
}

impl Amount {
    fn from_request(request: &BudgetRequest) -> Self {
        Self {
            tokens: request.max_tokens,
            cost_micro: request.max_cost_micro,
        }
    }

    fn can_afford(self, request: &BudgetRequest) -> bool {
        request
            .max_tokens
            .zip(self.tokens)
            .is_none_or(|(wanted, remaining)| wanted <= remaining)
            && request
                .max_cost_micro
                .zip(self.cost_micro)
                .is_none_or(|(wanted, remaining)| wanted <= remaining)
    }

    fn subtract(&mut self, request: &BudgetRequest) {
        if let (Some(remaining), Some(wanted)) = (&mut self.tokens, request.max_tokens) {
            *remaining -= wanted;
        }
        if let (Some(remaining), Some(wanted)) = (&mut self.cost_micro, request.max_cost_micro) {
            *remaining -= wanted;
        }
    }

    fn add(&mut self, amount: Self) {
        if let (Some(remaining), Some(value)) = (&mut self.tokens, amount.tokens) {
            *remaining = remaining.saturating_add(value);
        }
        if let (Some(remaining), Some(value)) = (&mut self.cost_micro, amount.cost_micro) {
            *remaining = remaining.saturating_add(value);
        }
    }

    fn unused(self, usage: &UsageReport) -> Self {
        Self {
            tokens: self
                .tokens
                .map(|reserved| reserved.saturating_sub(usage.tokens_used)),
            cost_micro: self
                .cost_micro
                .map(|reserved| reserved.saturating_sub(usage.cost_micro)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ScopeState {
    view: BudgetScope,
    remaining: Amount,
    reservation_id: Option<BudgetReservationId>,
    closed: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct BudgetState {
    scopes: HashMap<BudgetScopeId, ScopeState>,
    reservations: HashMap<BudgetReservationId, BudgetScopeId>,
    principal_roots: HashMap<String, BudgetScopeId>,
    legacy_chains: HashMap<BudgetReservationId, Vec<BudgetReservationId>>,
    operation_scopes: HashMap<OperationId, BudgetScopeId>,
    transfers: HashMap<BudgetReservationId, BudgetTransferReceipt>,
}

/// Thread-safe owner of all rollout budget hierarchies.
pub struct InMemoryBudgetController {
    state: Mutex<BudgetState>,
    durable_path: Option<PathBuf>,
}

/// Persistence-capable budget implementation used by daemon bootstrap. The
/// alias preserves the existing in-memory API while making durable intent
/// explicit at construction sites through `open_durable`.
pub type DurableBudgetController = InMemoryBudgetController;

impl std::fmt::Debug for InMemoryBudgetController {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InMemoryBudgetController")
            .finish_non_exhaustive()
    }
}

impl InMemoryBudgetController {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(BudgetState::default()),
            durable_path: None,
        }
    }

    /// Open a crash-durable controller. Every reservation/close/transfer is
    /// atomically persisted before its method returns.
    pub fn open_durable(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let state = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => BudgetState::default(),
            Err(error) => return Err(error),
        };
        Ok(Self {
            state: Mutex::new(state),
            durable_path: Some(path),
        })
    }

    fn persist(&self, state: &BudgetState) -> Result<(), AdmissionError> {
        let Some(path) = &self.durable_path else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(durable_error)?;
        }
        let bytes = serde_json::to_vec(state).map_err(durable_error)?;
        let temporary = path.with_extension("tmp");
        {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(&temporary)
                .map_err(durable_error)?;
            file.write_all(&bytes).map_err(durable_error)?;
            file.sync_all().map_err(durable_error)?;
        }
        std::fs::rename(&temporary, path).map_err(durable_error)?;
        if let Some(parent) = path.parent() {
            std::fs::File::open(parent)
                .and_then(|directory| directory.sync_all())
                .map_err(durable_error)?;
        }
        Ok(())
    }

    /// Create a versioned rollout root. Root identifiers are never inferred
    /// from principals, process ids, or operation ids.
    pub async fn create_root(
        &self,
        owner: impl Into<String>,
        limit: BudgetRequest,
    ) -> BudgetScopeId {
        let mut state = self.state.lock().await;
        let id = Self::create_root_locked(&mut state, owner.into(), limit);
        self.persist(&state)
            .expect("durable budget root persistence must succeed before publication");
        id
    }

    fn create_root_locked(
        state: &mut BudgetState,
        owner: String,
        limit: BudgetRequest,
    ) -> BudgetScopeId {
        let id = BudgetScopeId::new();
        state.scopes.insert(
            id,
            ScopeState {
                view: BudgetScope {
                    schema_version: BUDGET_SCOPE_SCHEMA_VERSION,
                    id,
                    parent: None,
                    kind: BudgetScopeKind::Rollout,
                    owner,
                    limit: limit.clone(),
                },
                remaining: Amount::from_request(&limit),
                reservation_id: None,
                closed: false,
            },
        );
        id
    }

    /// Atomically allocate a child from its direct parent.
    pub async fn reserve_child(
        &self,
        parent: BudgetScopeId,
        kind: BudgetScopeKind,
        owner: impl Into<String>,
        request: BudgetRequest,
    ) -> Result<BudgetReservationReceipt, AdmissionError> {
        let mut state = self.state.lock().await;
        let before = state.clone();
        let receipt = Self::reserve_child_locked(&mut state, parent, kind, owner.into(), request)?;
        if let Err(error) = self.persist(&state) {
            *state = before;
            return Err(error);
        }
        Ok(receipt)
    }

    fn reserve_child_locked(
        state: &mut BudgetState,
        parent: BudgetScopeId,
        kind: BudgetScopeKind,
        owner: String,
        request: BudgetRequest,
    ) -> Result<BudgetReservationReceipt, AdmissionError> {
        let parent_state = state
            .scopes
            .get_mut(&parent)
            .ok_or(AdmissionError::BudgetExceeded)?;
        if parent_state.closed
            || !parent_state.view.kind.accepts_child(kind)
            || !parent_state.remaining.can_afford(&request)
        {
            return Err(AdmissionError::BudgetExceeded);
        }
        parent_state.remaining.subtract(&request);
        let scope_id = BudgetScopeId::new();
        let reservation_id = BudgetReservationId::new();
        let receipt = BudgetReservationReceipt {
            reservation_id,
            scope_id,
            parent_scope_id: parent,
            request: request.clone(),
        };
        state.scopes.insert(
            scope_id,
            ScopeState {
                view: BudgetScope {
                    schema_version: BUDGET_SCOPE_SCHEMA_VERSION,
                    id: scope_id,
                    parent: Some(parent),
                    kind,
                    owner,
                    limit: request.clone(),
                },
                remaining: Amount::from_request(&request),
                reservation_id: Some(reservation_id),
                closed: false,
            },
        );
        state.reservations.insert(reservation_id, scope_id);
        Ok(receipt)
    }

    pub async fn scope(&self, id: BudgetScopeId) -> Option<BudgetScope> {
        self.state
            .lock()
            .await
            .scopes
            .get(&id)
            .map(|scope| scope.view.clone())
    }

    pub async fn remaining(&self, id: BudgetScopeId) -> Option<BudgetRequest> {
        self.state
            .lock()
            .await
            .scopes
            .get(&id)
            .filter(|scope| !scope.closed)
            .map(|scope| BudgetRequest {
                max_tokens: scope.remaining.tokens,
                max_cost_micro: scope.remaining.cost_micro,
            })
    }

    /// Close a leaf allocation and return its unused capacity to its parent.
    pub async fn settle_reservation(
        &self,
        reservation: BudgetReservationId,
        usage: &UsageReport,
    ) -> Result<(), AdmissionError> {
        let mut state = self.state.lock().await;
        let before = state.clone();
        Self::close_locked(&mut state, reservation, Some(usage))?;
        if let Err(error) = self.persist(&state) {
            *state = before;
            return Err(error);
        }
        Ok(())
    }

    pub async fn transfer_remaining_reservation(
        &self,
        child: BudgetReservationId,
        parent: BudgetReservationId,
        usage: &UsageReport,
    ) -> Result<BudgetTransferReceipt, AdmissionError> {
        let mut state = self.state.lock().await;
        let before = state.clone();
        if let Some(receipt) = state.transfers.get(&child) {
            return if receipt.parent_reservation_id == parent {
                Ok(receipt.clone())
            } else {
                Err(AdmissionError::BudgetExceeded)
            };
        }
        let child_scope_id = *state
            .reservations
            .get(&child)
            .ok_or(AdmissionError::AlreadySettled)?;
        let parent_scope_id = *state
            .reservations
            .get(&parent)
            .ok_or(AdmissionError::AlreadySettled)?;
        let (child_parent, accepted) = {
            let scope = state
                .scopes
                .get(&child_scope_id)
                .ok_or(AdmissionError::AlreadySettled)?;
            if scope.closed {
                return Err(AdmissionError::AlreadySettled);
            }
            (
                scope.view.parent,
                Amount::from_request(&scope.view.limit).unused(usage),
            )
        };
        let parent_state = state
            .scopes
            .get(&parent_scope_id)
            .ok_or(AdmissionError::AlreadySettled)?;
        if parent_state.closed || child == parent || parent_state.view.parent != child_parent {
            return Err(AdmissionError::BudgetExceeded);
        }
        state
            .scopes
            .get_mut(&child_scope_id)
            .expect("checked child scope")
            .closed = true;
        state
            .scopes
            .get_mut(&parent_scope_id)
            .expect("checked parent scope")
            .remaining
            .add(accepted);
        state.reservations.remove(&child);
        let receipt = BudgetTransferReceipt {
            child_reservation_id: child,
            parent_reservation_id: parent,
            accepted: BudgetRequest {
                max_tokens: accepted.tokens,
                max_cost_micro: accepted.cost_micro,
            },
        };
        state.transfers.insert(child, receipt.clone());
        if let Err(error) = self.persist(&state) {
            *state = before;
            return Err(error);
        }
        Ok(receipt)
    }

    pub async fn revoke_reservation(
        &self,
        reservation: BudgetReservationId,
    ) -> Result<(), AdmissionError> {
        let mut state = self.state.lock().await;
        let before = state.clone();
        Self::close_locked(&mut state, reservation, None)?;
        if let Err(error) = self.persist(&state) {
            *state = before;
            return Err(error);
        }
        Ok(())
    }

    /// Revoke a scope and every live descendant in post-order. Repeating the
    /// cleanup is a successful no-op, which makes terminal lifecycle retries
    /// safe after partial failure.
    pub async fn revoke_scope_tree(&self, scope_id: BudgetScopeId) -> Result<(), AdmissionError> {
        let mut state = self.state.lock().await;
        if state.scopes.get(&scope_id).is_none_or(|scope| scope.closed) {
            return Ok(());
        }
        let mut post_order = Vec::new();
        Self::descendants_post_order(&state, scope_id, &mut post_order);
        if state
            .scopes
            .get(&scope_id)
            .and_then(|scope| scope.reservation_id)
            .is_some()
        {
            post_order.push(scope_id);
        }
        for child in post_order {
            let reservation = state
                .scopes
                .get(&child)
                .and_then(|scope| scope.reservation_id);
            if let Some(reservation) = reservation {
                Self::close_locked(&mut state, reservation, None)?;
            }
        }
        Ok(())
    }

    fn descendants_post_order(
        state: &BudgetState,
        parent: BudgetScopeId,
        output: &mut Vec<BudgetScopeId>,
    ) {
        let children: Vec<_> = state
            .scopes
            .values()
            .filter(|scope| !scope.closed && scope.view.parent == Some(parent))
            .map(|scope| scope.view.id)
            .collect();
        for child in children {
            Self::descendants_post_order(state, child, output);
            output.push(child);
        }
    }

    pub async fn active_reservation_count(&self) -> usize {
        self.state.lock().await.reservations.len()
    }

    pub async fn bind_operation_scope(&self, operation: OperationId, scope: BudgetScopeId) {
        self.state
            .lock()
            .await
            .operation_scopes
            .insert(operation, scope);
    }

    pub async fn has_operation_scope(&self, operation: OperationId) -> bool {
        self.state
            .lock()
            .await
            .operation_scopes
            .contains_key(&operation)
    }

    pub async fn reserve_capability_for_operation(
        &self,
        operation: OperationId,
        permit: PermitId,
        request: BudgetRequest,
    ) -> Result<BudgetReservationReceipt, AdmissionError> {
        let mut state = self.state.lock().await;
        let parent = *state
            .operation_scopes
            .get(&operation)
            .ok_or(AdmissionError::BudgetExceeded)?;
        Self::reserve_child_locked(
            &mut state,
            parent,
            BudgetScopeKind::Capability,
            format!("permit:{}", permit.0),
            request,
        )
    }

    fn close_locked(
        state: &mut BudgetState,
        reservation: BudgetReservationId,
        usage: Option<&UsageReport>,
    ) -> Result<(), AdmissionError> {
        let scope_id = *state
            .reservations
            .get(&reservation)
            .ok_or(AdmissionError::AlreadySettled)?;
        if state
            .scopes
            .values()
            .any(|scope| !scope.closed && scope.view.parent == Some(scope_id))
        {
            return Err(AdmissionError::BudgetExceeded);
        }
        let (parent, returned) = {
            let scope = state
                .scopes
                .get_mut(&scope_id)
                .ok_or(AdmissionError::AlreadySettled)?;
            if scope.closed {
                return Err(AdmissionError::AlreadySettled);
            }
            scope.closed = true;
            let allocated = Amount::from_request(&scope.view.limit);
            (
                scope.view.parent.expect("non-root reservation has parent"),
                usage.map_or(allocated, |value| allocated.unused(value)),
            )
        };
        if let Some(parent_scope) = state.scopes.get_mut(&parent) {
            parent_scope.remaining.add(returned);
        }
        state.reservations.remove(&reservation);
        Ok(())
    }

    /// Compatibility adapter for existing admission callers. It is bounded to
    /// one principal rollout root and still accounts through all four levels.
    pub async fn set_budget(
        &self,
        principal: &str,
        max_tokens: Option<u64>,
        max_cost_micro: Option<u64>,
    ) {
        let mut state = self.state.lock().await;
        let root = Self::create_root_locked(
            &mut state,
            format!("principal:{principal}"),
            BudgetRequest {
                max_tokens,
                max_cost_micro,
            },
        );
        state.principal_roots.insert(principal.to_string(), root);
    }

    pub async fn reserve(
        &self,
        principal: &str,
        request: &BudgetRequest,
    ) -> Result<BudgetReservationId, AdmissionError> {
        let mut state = self.state.lock().await;
        let root = match state.principal_roots.get(principal).copied() {
            Some(root) => root,
            None => {
                let root = Self::create_root_locked(
                    &mut state,
                    format!("principal:{principal}"),
                    BudgetRequest {
                        max_tokens: None,
                        max_cost_micro: None,
                    },
                );
                state.principal_roots.insert(principal.to_string(), root);
                root
            }
        };
        let process = Self::reserve_child_locked(
            &mut state,
            root,
            BudgetScopeKind::Process,
            format!("compat-process:{principal}"),
            request.clone(),
        )?;
        let operation = Self::reserve_child_locked(
            &mut state,
            process.scope_id,
            BudgetScopeKind::Operation,
            format!("compat-operation:{principal}"),
            request.clone(),
        )?;
        let capability = Self::reserve_child_locked(
            &mut state,
            operation.scope_id,
            BudgetScopeKind::Capability,
            format!("compat-capability:{principal}"),
            request.clone(),
        )?;
        state.legacy_chains.insert(
            capability.reservation_id,
            vec![
                capability.reservation_id,
                operation.reservation_id,
                process.reservation_id,
            ],
        );
        Ok(capability.reservation_id)
    }

    pub async fn settle(
        &self,
        _principal: &str,
        reservation: BudgetReservationId,
        usage: &UsageReport,
    ) {
        let mut state = self.state.lock().await;
        if let Some(chain) = state.legacy_chains.remove(&reservation) {
            for id in chain {
                let _ = Self::close_locked(&mut state, id, Some(usage));
            }
        } else {
            let _ = Self::close_locked(&mut state, reservation, Some(usage));
        }
    }

    pub async fn revoke(&self, _principal: &str, reservation: BudgetReservationId) {
        let mut state = self.state.lock().await;
        if let Some(chain) = state.legacy_chains.remove(&reservation) {
            for id in chain {
                let _ = Self::close_locked(&mut state, id, None);
            }
        } else {
            let _ = Self::close_locked(&mut state, reservation, None);
        }
    }

    pub async fn remaining_tokens(&self, principal: &str) -> Option<Option<u64>> {
        let state = self.state.lock().await;
        let root = *state.principal_roots.get(principal)?;
        Some(state.scopes.get(&root)?.remaining.tokens)
    }

    pub async fn remaining_cost(&self, principal: &str) -> Option<Option<u64>> {
        let state = self.state.lock().await;
        let root = *state.principal_roots.get(principal)?;
        Some(state.scopes.get(&root)?.remaining.cost_micro)
    }
}

impl Default for InMemoryBudgetController {
    fn default() -> Self {
        Self::new()
    }
}

/// Boundary contract impl so the application layer can depend on
/// `Arc<dyn BudgetController>` instead of this concrete type. Each method
/// delegates to the inherent implementation.
#[async_trait]
impl BudgetController for InMemoryBudgetController {
    async fn create_root(&self, owner: String, limit: BudgetRequest) -> BudgetScopeId {
        InMemoryBudgetController::create_root(self, owner, limit).await
    }

    async fn reserve_child(
        &self,
        parent: BudgetScopeId,
        kind: BudgetScopeKind,
        owner: String,
        request: BudgetRequest,
    ) -> Result<BudgetReservationReceipt, AdmissionError> {
        InMemoryBudgetController::reserve_child(self, parent, kind, owner, request).await
    }

    async fn settle_reservation(
        &self,
        reservation: BudgetReservationId,
        usage: &UsageReport,
    ) -> Result<(), AdmissionError> {
        InMemoryBudgetController::settle_reservation(self, reservation, usage).await
    }

    async fn transfer_remaining_reservation(
        &self,
        child: BudgetReservationId,
        parent: BudgetReservationId,
        usage: &UsageReport,
    ) -> Result<BudgetTransferReceipt, AdmissionError> {
        InMemoryBudgetController::transfer_remaining_reservation(self, child, parent, usage).await
    }

    async fn revoke_reservation(
        &self,
        reservation: BudgetReservationId,
    ) -> Result<(), AdmissionError> {
        InMemoryBudgetController::revoke_reservation(self, reservation).await
    }

    async fn scope(&self, id: BudgetScopeId) -> Option<BudgetScope> {
        InMemoryBudgetController::scope(self, id).await
    }

    async fn active_reservation_count(&self) -> usize {
        InMemoryBudgetController::active_reservation_count(self).await
    }

    async fn reservation_for_owner(&self, owner: &str) -> Option<BudgetReservationId> {
        self.state
            .lock()
            .await
            .scopes
            .values()
            .find(|scope| scope.view.owner == owner)
            .and_then(|scope| scope.reservation_id)
    }

    async fn transfer_for_child(
        &self,
        child: BudgetReservationId,
    ) -> Option<BudgetTransferReceipt> {
        self.state.lock().await.transfers.get(&child).cloned()
    }
}

fn durable_error(error: impl std::fmt::Display) -> AdmissionError {
    AdmissionError::Denied {
        reason: format!("durable budget persistence failed: {error}"),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn durable_transfer_survives_crash_windows_and_replays_after_reopen() {
        let path = std::env::temp_dir().join(format!(
            "aletheon-budget-transfer-{}.json",
            uuid::Uuid::new_v4()
        ));
        let (child, parent, usage, first) = {
            let controller = InMemoryBudgetController::open_durable(&path).unwrap();
            let root = controller
                .create_root(
                    "root",
                    BudgetRequest {
                        max_tokens: Some(100),
                        max_cost_micro: Some(100),
                    },
                )
                .await;
            let parent = controller
                .reserve_child(
                    root,
                    BudgetScopeKind::Process,
                    "parent",
                    BudgetRequest {
                        max_tokens: Some(40),
                        max_cost_micro: Some(40),
                    },
                )
                .await
                .unwrap();
            let child = controller
                .reserve_child(
                    root,
                    BudgetScopeKind::Process,
                    "child",
                    BudgetRequest {
                        max_tokens: Some(30),
                        max_cost_micro: Some(30),
                    },
                )
                .await
                .unwrap();
            let usage = UsageReport {
                tokens_used: 10,
                cost_micro: 5,
                ..Default::default()
            };
            let first = controller
                .transfer_remaining_reservation(child.reservation_id, parent.reservation_id, &usage)
                .await
                .unwrap();
            (child, parent, usage, first)
        };
        let reopened = InMemoryBudgetController::open_durable(&path).unwrap();
        let replay = reopened
            .transfer_remaining_reservation(child.reservation_id, parent.reservation_id, &usage)
            .await
            .unwrap();
        assert_eq!(replay, first);
        assert_eq!(reopened.active_reservation_count().await, 1);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn durable_untransferred_reservation_is_reclaimed_after_reopen() {
        let path = std::env::temp_dir().join(format!(
            "aletheon-budget-reclaim-{}.json",
            uuid::Uuid::new_v4()
        ));
        let reservation = {
            let controller = InMemoryBudgetController::open_durable(&path).unwrap();
            let root = controller
                .create_root(
                    "root",
                    BudgetRequest {
                        max_tokens: Some(100),
                        max_cost_micro: Some(100),
                    },
                )
                .await;
            controller
                .reserve_child(
                    root,
                    BudgetScopeKind::Process,
                    "agent:recover",
                    BudgetRequest {
                        max_tokens: Some(30),
                        max_cost_micro: Some(30),
                    },
                )
                .await
                .unwrap()
                .reservation_id
        };
        let reopened = InMemoryBudgetController::open_durable(&path).unwrap();
        assert_eq!(
            BudgetController::reservation_for_owner(&reopened, "agent:recover").await,
            Some(reservation)
        );
        reopened.revoke_reservation(reservation).await.unwrap();
        drop(reopened);
        let replay = InMemoryBudgetController::open_durable(&path).unwrap();
        assert_eq!(replay.active_reservation_count().await, 0);
        assert_eq!(
            replay.revoke_reservation(reservation).await,
            Err(AdmissionError::AlreadySettled)
        );
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn remaining_reservation_transfer_is_atomic_and_replay_stable() {
        let ctrl = InMemoryBudgetController::new();
        let root = ctrl
            .create_root(
                "root",
                BudgetRequest {
                    max_tokens: Some(100),
                    max_cost_micro: Some(100),
                },
            )
            .await;
        let parent = ctrl
            .reserve_child(
                root,
                BudgetScopeKind::Process,
                "parent",
                BudgetRequest {
                    max_tokens: Some(40),
                    max_cost_micro: Some(40),
                },
            )
            .await
            .unwrap();
        let child = ctrl
            .reserve_child(
                root,
                BudgetScopeKind::Process,
                "child",
                BudgetRequest {
                    max_tokens: Some(30),
                    max_cost_micro: Some(30),
                },
            )
            .await
            .unwrap();
        let usage = UsageReport {
            tokens_used: 10,
            cost_micro: 5,
            ..Default::default()
        };
        let first = ctrl
            .transfer_remaining_reservation(child.reservation_id, parent.reservation_id, &usage)
            .await
            .unwrap();
        let replay = ctrl
            .transfer_remaining_reservation(child.reservation_id, parent.reservation_id, &usage)
            .await
            .unwrap();
        assert_eq!(first, replay);
        assert_eq!(first.accepted.max_tokens, Some(20));
        assert_eq!(first.accepted.max_cost_micro, Some(25));
        assert_eq!(ctrl.active_reservation_count().await, 1);
    }

    #[tokio::test]
    async fn unlimited_account_always_approves() {
        let ctrl = InMemoryBudgetController::new();
        let req = BudgetRequest {
            max_tokens: Some(10_000),
            max_cost_micro: Some(500),
        };
        let id = ctrl.reserve("agent-1", &req).await.unwrap();
        // settle
        ctrl.settle(
            "agent-1",
            id,
            &UsageReport {
                tokens_used: 8_000,
                cost_micro: 400,
                ..Default::default()
            },
        )
        .await;
    }

    #[tokio::test]
    async fn limited_account_approves_within_budget() {
        let ctrl = InMemoryBudgetController::new();
        ctrl.set_budget("agent-1", Some(100_000), Some(1_000_000))
            .await;

        let req = BudgetRequest {
            max_tokens: Some(10_000),
            max_cost_micro: Some(5_000),
        };
        let id = ctrl.reserve("agent-1", &req).await.unwrap();
        assert!(id.0 != uuid::Uuid::nil());
    }

    #[tokio::test]
    async fn limited_account_denies_over_budget() {
        let ctrl = InMemoryBudgetController::new();
        ctrl.set_budget("agent-1", Some(1_000), Some(10_000)).await;

        // Request more tokens than the budget allows.
        let req = BudgetRequest {
            max_tokens: Some(5_000),
            max_cost_micro: None,
        };
        let err = ctrl.reserve("agent-1", &req).await.unwrap_err();
        assert!(matches!(err, AdmissionError::BudgetExceeded));
    }

    #[tokio::test]
    async fn settle_adjusts_to_actual_usage() {
        let ctrl = InMemoryBudgetController::new();
        ctrl.set_budget("agent-1", Some(100_000), None).await;

        // Reserve 10k tokens, only use 3k — remaining should reflect actual.
        let req = BudgetRequest {
            max_tokens: Some(10_000),
            max_cost_micro: None,
        };
        let id = ctrl.reserve("agent-1", &req).await.unwrap();
        ctrl.settle(
            "agent-1",
            id,
            &UsageReport {
                tokens_used: 3_000,
                ..Default::default()
            },
        )
        .await;

        // 100_000 - 3_000 = 97_000 remaining
        // After settle, we should be able to request up to 97_000.
        let req2 = BudgetRequest {
            max_tokens: Some(97_000),
            max_cost_micro: None,
        };
        let id2 = ctrl.reserve("agent-1", &req2).await.unwrap();
        ctrl.settle(
            "agent-1",
            id2,
            &UsageReport {
                tokens_used: 97_000,
                ..Default::default()
            },
        )
        .await;

        // Now budget should be exhausted.
        let req3 = BudgetRequest {
            max_tokens: Some(1),
            max_cost_micro: None,
        };
        let err = ctrl.reserve("agent-1", &req3).await.unwrap_err();
        assert!(matches!(err, AdmissionError::BudgetExceeded));
    }

    #[tokio::test]
    async fn revoke_returns_budget_to_pool() {
        let ctrl = InMemoryBudgetController::new();
        ctrl.set_budget("agent-1", Some(10_000), None).await;

        // Reserve 5k, then revoke.
        let req = BudgetRequest {
            max_tokens: Some(5_000),
            max_cost_micro: None,
        };
        let id = ctrl.reserve("agent-1", &req).await.unwrap();
        ctrl.revoke("agent-1", id).await;

        // Should be able to reserve 10k again (full budget returned).
        let req2 = BudgetRequest {
            max_tokens: Some(10_000),
            max_cost_micro: None,
        };
        let id2 = ctrl.reserve("agent-1", &req2).await.unwrap();
        assert!(id2.0 != uuid::Uuid::nil());
    }

    #[tokio::test]
    async fn cost_budget_limits_enforced() {
        let ctrl = InMemoryBudgetController::new();
        ctrl.set_budget("agent-1", None, Some(1_000)).await;

        let req = BudgetRequest {
            max_tokens: None,
            max_cost_micro: Some(2_000),
        };
        let err = ctrl.reserve("agent-1", &req).await.unwrap_err();
        assert!(matches!(err, AdmissionError::BudgetExceeded));
    }
}
