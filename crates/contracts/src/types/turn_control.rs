//! Turn interruption vocabulary shared by harnesses and their hosts.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterruptReason {
    UserCancelled,
    Timeout,
    BudgetExceeded,
}
