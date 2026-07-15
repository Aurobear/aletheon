//! Narrow request-facing use-case ports.

use std::sync::Arc;

use mnemosyne::FactUseCases;

use crate::service::GoalUseCases;

pub(crate) struct HandlerPorts {
    pub(crate) facts: Arc<dyn FactUseCases>,
    pub(crate) goals: Arc<dyn GoalUseCases>,
}

impl HandlerPorts {
    pub(crate) fn new(facts: Arc<dyn FactUseCases>, goals: Arc<dyn GoalUseCases>) -> Self {
        Self { facts, goals }
    }
}
