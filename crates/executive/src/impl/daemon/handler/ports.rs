//! Narrow request-facing use-case ports.

use std::sync::Arc;

use mnemosyne::FactUseCases;

use crate::service::{ApprovalUseCases, GoalUseCases};

pub(crate) struct HandlerPorts {
    pub(crate) facts: Arc<dyn FactUseCases>,
    pub(crate) goals: Arc<dyn GoalUseCases>,
    pub(crate) approvals: Arc<dyn ApprovalUseCases>,
}

impl HandlerPorts {
    pub(crate) fn new(
        facts: Arc<dyn FactUseCases>,
        goals: Arc<dyn GoalUseCases>,
        approvals: Arc<dyn ApprovalUseCases>,
    ) -> Self {
        Self {
            facts,
            goals,
            approvals,
        }
    }
}
