//! Human-in-the-loop approval gate for risky tool execution.
//!
//! Decouples the approval *decision channel* (terminal, socket, auto) from the
//! security runner. The runner asks the gate before executing L2+ tools; the
//! gate returns the user's decision. Fail-safe: any error/timeout upstream
//! should map to `Deny`.

use async_trait::async_trait;
use fabric::{ApprovalOwner, ConnectionId, TurnId, WorkspacePolicy};

/// A request for the user to approve a single tool action.
#[derive(Debug, Clone)]
pub struct ApprovalRequest {
    pub owner: ApprovalOwner,
    pub connection_id: ConnectionId,
    pub turn_id: TurnId,
    pub call_id: String,
    pub workspace: WorkspacePolicy,
    /// Tool name, e.g. "bash_exec".
    pub tool: String,
    /// One-line human-readable summary, e.g. "bash: rm -rf /tmp/x".
    pub action_summary: String,
    /// Risk descriptor, e.g. "low" | "medium" | "high".
    pub risk_level: String,
    /// Optional full command / diff for the user to inspect.
    pub detail: Option<String>,
}

/// The user's decision on an approval request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalDecision {
    /// Execute this action once.
    Approve,
    /// Reject this action.
    Deny,
    /// Approve this action and auto-approve the same tool for the rest of the session.
    ApproveForSession,
}

/// Abstraction over how approval is requested from the user.
///
/// Implementations: `TerminalApprovalGate` (stdin/stdout y/n), `AutoApproveGate` /
/// `AutoDenyGate` (tests and conservative defaults). A socket/TUI gate arrives in
/// Phase 2.
#[async_trait]
pub trait ApprovalGate: Send + Sync {
    /// Request a decision for the given action. Implementations must never panic;
    /// on any internal failure they should return `ApprovalDecision::Deny`.
    async fn request(&self, req: &ApprovalRequest) -> ApprovalDecision;
}

/// Always approves. For tests and trusted/automated contexts.
pub struct AutoApproveGate;

#[async_trait]
impl ApprovalGate for AutoApproveGate {
    async fn request(&self, _req: &ApprovalRequest) -> ApprovalDecision {
        ApprovalDecision::Approve
    }
}

/// Always denies. The conservative default (preserves current "deny L2+ in automated
/// mode" behavior) and a test double.
pub struct AutoDenyGate;

#[async_trait]
impl ApprovalGate for AutoDenyGate {
    async fn request(&self, _req: &ApprovalRequest) -> ApprovalDecision {
        ApprovalDecision::Deny
    }
}

/// Approval gate that prompts on the controlling terminal (stdin/stdout).
///
/// Used by the single-process `aletheon-exec` path. Reads one line:
/// `y` = Approve, `a` = ApproveForSession, anything else (incl. EOF) = Deny (fail-safe).
pub struct TerminalApprovalGate;

#[async_trait]
impl ApprovalGate for TerminalApprovalGate {
    async fn request(&self, req: &ApprovalRequest) -> ApprovalDecision {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

        let mut stdout = tokio::io::stdout();
        let prompt = format!(
            "\n\u{26a0}  Approval required [{}] {}\n   {}\n   Approve? [y]es / [a]lways / [N]o: ",
            req.risk_level, req.tool, req.action_summary,
        );
        if stdout.write_all(prompt.as_bytes()).await.is_err() {
            return ApprovalDecision::Deny;
        }
        let _ = stdout.flush().await;

        let mut line = String::new();
        let mut reader = BufReader::new(tokio::io::stdin());
        match reader.read_line(&mut line).await {
            Ok(0) | Err(_) => ApprovalDecision::Deny, // EOF or error -> fail-safe deny
            Ok(_) => match line.trim().to_lowercase().as_str() {
                "y" | "yes" => ApprovalDecision::Approve,
                "a" | "always" => ApprovalDecision::ApproveForSession,
                _ => ApprovalDecision::Deny,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn auto_deny_gate_denies() {
        let gate = AutoDenyGate;
        let req = ApprovalRequest {
            owner: fabric::ApprovalOwner::new(
                fabric::PrincipalId("test".into()),
                fabric::ThreadId("test".into()),
            ),
            connection_id: fabric::ConnectionId::new(),
            turn_id: fabric::TurnId::new(),
            call_id: "call".into(),
            workspace: fabric::WorkspacePolicy::from_resolved_roots("/tmp".into(), vec![]).unwrap(),
            tool: "bash_exec".into(),
            action_summary: "rm -rf /tmp/x".into(),
            risk_level: "high".into(),
            detail: None,
        };
        assert_eq!(gate.request(&req).await, ApprovalDecision::Deny);
    }

    #[tokio::test]
    async fn auto_approve_gate_approves() {
        let gate = AutoApproveGate;
        let req = ApprovalRequest {
            owner: fabric::ApprovalOwner::new(
                fabric::PrincipalId("test".into()),
                fabric::ThreadId("test".into()),
            ),
            connection_id: fabric::ConnectionId::new(),
            turn_id: fabric::TurnId::new(),
            call_id: "call".into(),
            workspace: fabric::WorkspacePolicy::from_resolved_roots("/tmp".into(), vec![]).unwrap(),
            tool: "file_write".into(),
            action_summary: "write hello.txt".into(),
            risk_level: "low".into(),
            detail: None,
        };
        assert_eq!(gate.request(&req).await, ApprovalDecision::Approve);
    }
}
