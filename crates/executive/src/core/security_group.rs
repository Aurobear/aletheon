//! Security group — tool runner, storm breaker, and approval channels.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{mpsc, oneshot, Mutex};

use corpus::security::approval::ApprovalDecision;
use corpus::security::runner::ToolRunnerWithGuard;
use corpus::security::socket_approval::PendingApproval;
use corpus::security::storm_breaker::StormBreaker;

pub(crate) type ToolRunnerHandle = Arc<Mutex<ToolRunnerWithGuard>>;

pub(crate) struct SecurityGroup {
    pub(crate) tool_runner: ToolRunnerHandle,
    pub(crate) storm_breaker: Arc<Mutex<StormBreaker>>,
    pub(crate) approval_rx: Arc<Mutex<mpsc::Receiver<PendingApproval>>>,
    pub(crate) pending_approvals: Arc<Mutex<HashMap<String, oneshot::Sender<ApprovalDecision>>>>,
    pub(crate) session_approvals: Arc<Mutex<HashMap<String, bool>>>,
}
