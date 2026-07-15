//! Narrow request-facing use-case ports.

use std::sync::Arc;

use mnemosyne::FactUseCases;

pub(crate) struct HandlerPorts {
    pub(crate) facts: Arc<dyn FactUseCases>,
}

impl HandlerPorts {
    pub(crate) fn new(facts: Arc<dyn FactUseCases>) -> Self {
        Self { facts }
    }
}
