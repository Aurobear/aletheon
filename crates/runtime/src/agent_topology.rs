//! Runtime-owned directional Agent topology policy (RA-05).
//!
//! Host/application code authenticates the durable Agent records and asks this
//! policy to remember an explicitly permitted sibling route. The policy owns
//! only the bounded route state; it does not inspect Kernel or persistence and
//! therefore cannot become a second authorization authority.

use ::contracts::AgentId;
use parking_lot::RwLock;
use std::collections::HashSet;

#[derive(Default)]
pub struct AgentTopologyRoutes {
    routes: RwLock<HashSet<(AgentId, AgentId, AgentId)>>,
}

impl AgentTopologyRoutes {
    pub fn permit(&self, parent: AgentId, from: AgentId, to: AgentId) {
        self.routes.write().insert((parent, from, to));
    }

    pub fn is_permitted(&self, parent: AgentId, from: AgentId, to: AgentId) -> bool {
        self.routes.read().contains(&(parent, from, to))
    }

    pub fn revoke(&self, parent: AgentId, from: AgentId, to: AgentId) -> bool {
        self.routes.write().remove(&(parent, from, to))
    }

    pub fn len(&self) -> usize {
        self.routes.read().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_are_directional_and_idempotent() {
        let routes = AgentTopologyRoutes::default();
        let parent = AgentId::new();
        let from = AgentId::new();
        let to = AgentId::new();
        routes.permit(parent, from, to);
        routes.permit(parent, from, to);
        assert_eq!(routes.len(), 1);
        assert!(routes.is_permitted(parent, from, to));
        assert!(!routes.is_permitted(parent, to, from));
        assert!(routes.revoke(parent, from, to));
        assert!(!routes.is_permitted(parent, from, to));
    }
}
