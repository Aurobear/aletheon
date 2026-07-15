//! Narrow request-facing use-case ports.

use std::sync::Arc;

use mnemosyne::FactUseCases;

use crate::service::legacy_session_service::LegacySessionUseCases;
use crate::service::{AdminUseCases, ApprovalUseCases, GoalUseCases};

pub(crate) struct HandlerPorts {
    pub(crate) facts: Arc<dyn FactUseCases>,
    pub(crate) goals: Arc<dyn GoalUseCases>,
    pub(crate) approvals: Arc<dyn ApprovalUseCases>,
    pub(crate) admin: Arc<dyn AdminUseCases>,
    pub(crate) sessions: Arc<dyn LegacySessionUseCases>,
}

impl HandlerPorts {
    pub(crate) fn new(
        facts: Arc<dyn FactUseCases>,
        goals: Arc<dyn GoalUseCases>,
        approvals: Arc<dyn ApprovalUseCases>,
        admin: Arc<dyn AdminUseCases>,
        sessions: Arc<dyn LegacySessionUseCases>,
    ) -> Self {
        Self {
            facts,
            goals,
            approvals,
            admin,
            sessions,
        }
    }
}
