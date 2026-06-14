//! IdentityLayer — current identity model + mutation history.
//!
//! The identity is the agent's self-model. Every mutation preserves
//! the previous state in a history chain.

use aletheon_abi::Identity;
use chrono::Utc;
use parking_lot::RwLock;

/// Record of a past identity state.
#[derive(Debug, Clone)]
pub struct IdentityRecord {
    pub identity: Identity,
    pub mutated_at: chrono::DateTime<chrono::Utc>,
    pub reason: String,
}

/// IdentityLayer — holds current identity and a history of past identities.
pub struct IdentityLayer {
    current: RwLock<Identity>,
    history: RwLock<Vec<IdentityRecord>>,
}

impl IdentityLayer {
    pub fn new(name: impl Into<String>, description: impl Into<String>, version: impl Into<String>) -> Self {
        let identity = Identity {
            name: name.into(),
            description: description.into(),
            version: version.into(),
            created_at: Utc::now(),
            last_mutation: None,
        };
        Self {
            current: RwLock::new(identity),
            history: RwLock::new(Vec::new()),
        }
    }

    /// Get the current identity.
    pub fn current(&self) -> Identity {
        self.current.read().clone()
    }

    /// Apply a mutation. The previous identity is pushed to history.
    pub fn mutate(
        &self,
        new_name: Option<String>,
        new_description: Option<String>,
        new_version: Option<String>,
        reason: impl Into<String>,
    ) -> Identity {
        let mut current = self.current.write();
        let old = current.clone();

        // Push old to history
        self.history.write().push(IdentityRecord {
            identity: old,
            mutated_at: Utc::now(),
            reason: reason.into(),
        });

        // Apply changes
        if let Some(name) = new_name {
            current.name = name;
        }
        if let Some(desc) = new_description {
            current.description = desc;
        }
        if let Some(ver) = new_version {
            current.version = ver;
        }
        current.last_mutation = Some(Utc::now());

        current.clone()
    }

    /// Get mutation history (oldest first).
    pub fn history(&self) -> Vec<IdentityRecord> {
        self.history.read().clone()
    }

    /// Number of mutations applied.
    pub fn mutation_count(&self) -> usize {
        self.history.read().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_identity() {
        let layer = IdentityLayer::new("aurb", "An AI agent", "0.1.0");
        let id = layer.current();
        assert_eq!(id.name, "aurb");
        assert_eq!(id.description, "An AI agent");
        assert_eq!(id.version, "0.1.0");
        assert!(id.last_mutation.is_none());
        assert_eq!(layer.mutation_count(), 0);
    }

    #[test]
    fn mutate_preserves_history() {
        let layer = IdentityLayer::new("aurb", "desc", "0.1.0");
        let updated = layer.mutate(
            Some("aurb-v2".to_string()),
            None,
            Some("0.2.0".to_string()),
            "upgraded",
        );

        assert_eq!(updated.name, "aurb-v2");
        assert_eq!(updated.version, "0.2.0");
        assert!(updated.last_mutation.is_some());

        let history = layer.history();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].identity.name, "aurb");
        assert_eq!(history[0].identity.version, "0.1.0");
        assert_eq!(history[0].reason, "upgraded");
    }

    #[test]
    fn multiple_mutations_chain() {
        let layer = IdentityLayer::new("v0", "desc", "0.0.1");

        layer.mutate(Some("v1".to_string()), None, None, "step1");
        layer.mutate(Some("v2".to_string()), None, None, "step2");
        layer.mutate(Some("v3".to_string()), None, None, "step3");

        assert_eq!(layer.current().name, "v3");
        assert_eq!(layer.mutation_count(), 3);

        let history = layer.history();
        assert_eq!(history[0].identity.name, "v0");
        assert_eq!(history[1].identity.name, "v1");
        assert_eq!(history[2].identity.name, "v2");
    }
}
