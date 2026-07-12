//! Security group — tool runner, storm breaker, and approval channels.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{mpsc, oneshot, Mutex};

use corpus::security::security::approval::ApprovalDecision;
use corpus::security::security::runner::ToolRunnerWithGuard;
use corpus::security::security::socket_approval::PendingApproval;
use corpus::security::storm_breaker::StormBreaker;

pub struct SecurityGroup {
    pub tool_runner: Arc<Mutex<ToolRunnerWithGuard>>,
    pub storm_breaker: Arc<Mutex<StormBreaker>>,
    pub approval_rx: Arc<Mutex<mpsc::Receiver<PendingApproval>>>,
    pub pending_approvals: Arc<Mutex<HashMap<String, oneshot::Sender<ApprovalDecision>>>>,
    pub session_approvals: Arc<Mutex<HashMap<String, bool>>>,
}
