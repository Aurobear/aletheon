//! Subagent resource settlement types (G6).
//!
//! When a child agent completes, its resources are settled deterministically:
//! release leases, flush usage/budget, reparent or kill background resources,
//! promote/discard memory drafts — idempotently. This module holds the pure
//! types plus the reparent rule and the idempotency-key derivation. The
//! settlement state-machine orchestration lives in the AgentControl service.
//!
//! See `docs/plans/grok/exec/G6-subagent-settlement.md`.

use serde::{Deserialize, Serialize};

/// Resource categories; the class drives the default disposition at settlement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentResourceClass {
    /// Foreground command: must settle (await or cancel) before child exit.
    ForegroundCommand,
    /// Background command: may kill / reparent / detach.
    BackgroundCommand,
    /// Notification route: child-specific; on exit switch to parent or durable mailbox.
    NotificationRoute,
    /// Worktree: child-owned; clean / retain artifact / recovery.
    Worktree,
}

/// Disposition applied to a background resource at child exit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BackgroundDisposition {
    Kill,
    Reparent,
    Detach,
}

/// Settlement state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SettlementPhase {
    Running,
    /// Stop new calls, cancel/await foreground, classify background, flush.
    Quiescing,
    /// Reparent authorized survivors, release leases/reservations, persist recovery.
    Settling,
    Terminal,
}

/// Terminal classification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SettlementTerminal {
    Completed,
    Failed { reason: String },
    Cancelled,
    Recoverable,
}

/// A background resource declared at spawn. Only resources declared
/// `survive_child = true` are eligible for reparent (not a child-side
/// end-of-life self-promotion).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackgroundResourceDecl {
    pub resource_id: String,
    pub class: AgentResourceClass,
    pub survive_child: bool,
}

/// Immutable reparent receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReparentReceipt {
    pub resource_id: String,
    pub class: AgentResourceClass,
    pub old_owner: String,
    pub new_owner: String,
    pub reason: String,
    pub at_ms: i64,
}

/// Idempotent settlement receipt. Prevents double release / double promotion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettlementReceipt {
    pub agent_id: String,
    pub attempt_id: String,
    /// Daemon generation, for crash recovery.
    pub generation: String,
    pub terminal: SettlementTerminal,
    pub released_leases: Vec<String>,
    pub reparented: Vec<ReparentReceipt>,
    pub settled_at_ms: i64,
    /// Deterministic `agent_id + attempt_id + generation` identity. A durable
    /// store uses this key to return the first receipt on replay rather than
    /// repeating any side effect.
    pub idempotency_key: String,
}

/// Build the canonical settlement idempotency key.
pub fn settlement_idempotency_key(agent_id: &str, attempt_id: &str, generation: &str) -> String {
    format!("{agent_id}:{attempt_id}:{generation}")
}

/// Inputs for the reparent decision (all trusted, host-supplied).
#[derive(Debug, Clone)]
pub struct ReparentContext {
    /// Parent's workspace/capability authority covers the resource.
    pub parent_authority_covers: bool,
    /// Parent budget accepts the remaining cost/time reservation.
    pub parent_budget_accepts: bool,
    /// Notification route can switch to parent or a durable mailbox.
    pub notification_route_transferable: bool,
}

/// Pure reparent rule. A background resource may reparent only if ALL hold:
/// 1. declared `survive_child = true`;
/// 2. parent authority covers it;
/// 3. parent budget accepts remaining reservation;
/// 4. (notification routes only) route is transferable.
///
/// Otherwise it must be killed/detached, with the reason recorded.
pub fn can_reparent(decl: &BackgroundResourceDecl, ctx: &ReparentContext) -> Result<(), String> {
    if !decl.survive_child {
        return Err("resource was not declared survive_child".to_string());
    }
    if !ctx.parent_authority_covers {
        return Err("parent authority does not cover the resource".to_string());
    }
    if !ctx.parent_budget_accepts {
        return Err("parent budget rejects the remaining reservation".to_string());
    }
    if decl.class == AgentResourceClass::NotificationRoute && !ctx.notification_route_transferable {
        return Err("notification route is not transferable".to_string());
    }
    Ok(())
}

/// Upper bound on tracked background resources per agent.
pub const MAX_BACKGROUND_RESOURCES: usize = 64;

#[cfg(test)]
mod tests {
    use super::*;

    fn decl(class: AgentResourceClass, survive: bool) -> BackgroundResourceDecl {
        BackgroundResourceDecl {
            resource_id: "r1".to_string(),
            class,
            survive_child: survive,
        }
    }

    fn ctx_all_ok() -> ReparentContext {
        ReparentContext {
            parent_authority_covers: true,
            parent_budget_accepts: true,
            notification_route_transferable: true,
        }
    }

    #[test]
    fn reparent_allowed_when_all_conditions_hold() {
        let d = decl(AgentResourceClass::BackgroundCommand, true);
        assert!(can_reparent(&d, &ctx_all_ok()).is_ok());
    }

    #[test]
    fn reparent_denied_without_survive_child() {
        let d = decl(AgentResourceClass::BackgroundCommand, false);
        assert!(can_reparent(&d, &ctx_all_ok()).is_err());
    }

    #[test]
    fn reparent_denied_without_authority() {
        let d = decl(AgentResourceClass::BackgroundCommand, true);
        let mut ctx = ctx_all_ok();
        ctx.parent_authority_covers = false;
        assert!(can_reparent(&d, &ctx).is_err());
    }

    #[test]
    fn reparent_denied_when_budget_rejects() {
        let d = decl(AgentResourceClass::BackgroundCommand, true);
        let mut ctx = ctx_all_ok();
        ctx.parent_budget_accepts = false;
        assert!(can_reparent(&d, &ctx).is_err());
    }

    #[test]
    fn notification_route_needs_transferable() {
        let d = decl(AgentResourceClass::NotificationRoute, true);
        let mut ctx = ctx_all_ok();
        ctx.notification_route_transferable = false;
        assert!(can_reparent(&d, &ctx).is_err());
        // A non-route resource is unaffected by the transferable flag.
        let cmd = decl(AgentResourceClass::BackgroundCommand, true);
        assert!(can_reparent(&cmd, &ctx).is_ok());
    }

    #[test]
    fn idempotency_key_is_stable_and_distinct() {
        let base = SettlementReceipt {
            agent_id: "a1".to_string(),
            attempt_id: "att1".to_string(),
            generation: "gen1".to_string(),
            terminal: SettlementTerminal::Completed,
            released_leases: vec![],
            reparented: vec![],
            settled_at_ms: 0,
            idempotency_key: settlement_idempotency_key("a1", "att1", "gen1"),
        };
        assert_eq!(base.idempotency_key, "a1:att1:gen1");
        let mut other = base.clone();
        other.generation = "gen2".to_string();
        other.idempotency_key = settlement_idempotency_key("a1", "att1", "gen2");
        assert_ne!(base.idempotency_key, other.idempotency_key);
    }

    #[test]
    fn settlement_serde_roundtrip() {
        let r = SettlementReceipt {
            agent_id: "a1".to_string(),
            attempt_id: "att1".to_string(),
            generation: "gen1".to_string(),
            terminal: SettlementTerminal::Failed {
                reason: "boom".to_string(),
            },
            released_leases: vec!["lease-a".to_string()],
            reparented: vec![ReparentReceipt {
                resource_id: "r1".to_string(),
                class: AgentResourceClass::BackgroundCommand,
                old_owner: "child".to_string(),
                new_owner: "parent".to_string(),
                reason: "survivor".to_string(),
                at_ms: 5,
            }],
            settled_at_ms: 9,
            idempotency_key: settlement_idempotency_key("a1", "att1", "gen1"),
        };
        let json = serde_json::to_string(&r).unwrap();
        let back: SettlementReceipt = serde_json::from_str(&json).unwrap();
        assert_eq!(r, back);
    }
}
