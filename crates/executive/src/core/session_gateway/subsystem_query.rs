//! Subsystem query trait — per-module structured state export.
//!
//! Each subsystem (CoreMemory, SelfField, DaseinModule, etc.) implements
//! [`SubsystemQuery`] and registers itself with [`SubsystemRegistry`].
//! The Session Gateway routes `session.memory`, `session.self`, etc.
//! through the registry.

use serde_json::Value;
use std::collections::HashMap;

/// Error returned from a subsystem query.
#[derive(Debug)]
pub struct QueryError {
    pub code: i32,
    pub message: String,
}

impl QueryError {
    pub fn not_found(id: &str) -> Self {
        Self {
            code: -32051,
            message: format!("Unknown subsystem: {}", id),
        }
    }
}

/// Trait implemented by subsystems that can export their state.
///
/// The `query()` method returns markdown — the Session Gateway's universal
/// output format (compact for both humans and Claude to read).
pub trait SubsystemQuery: Send + Sync {
    /// Unique subsystem identifier (e.g., "memory.core", "self.boundary").
    fn subsystem_id(&self) -> &'static str;

    /// Export this subsystem's current state as markdown.
    ///
    /// `params` — optional query parameters (e.g., layer name, limit, filter).
    fn query(&self, params: &Value) -> Result<String, QueryError>;
}

/// Registry of [`SubsystemQuery`] implementations.
///
/// The Session Gateway looks up subsystems by ID at dispatch time.
pub struct SubsystemRegistry {
    subsystems: HashMap<&'static str, Box<dyn SubsystemQuery>>,
}

impl Default for SubsystemRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl SubsystemRegistry {
    pub fn new() -> Self {
        Self {
            subsystems: HashMap::new(),
        }
    }

    /// Register a subsystem for querying.
    pub fn register(&mut self, subsystem: Box<dyn SubsystemQuery>) {
        let id = subsystem.subsystem_id();
        self.subsystems.insert(id, subsystem);
    }

    /// Query a subsystem by its identifier.
    pub fn query(&self, id: &str, params: &Value) -> Result<String, QueryError> {
        match self.subsystems.get(id) {
            Some(s) => s.query(params),
            None => Err(QueryError::not_found(id)),
        }
    }

    /// List all registered subsystem IDs.
    pub fn list_ids(&self) -> Vec<&'static str> {
        self.subsystems.keys().copied().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestSubsystem {
        id: &'static str,
        state: String,
    }

    impl SubsystemQuery for TestSubsystem {
        fn subsystem_id(&self) -> &'static str {
            self.id
        }

        fn query(&self, _params: &Value) -> Result<String, QueryError> {
            Ok(format!("# {}\n\n{}\n", self.id, self.state))
        }
    }

    #[test]
    fn register_and_query() {
        let mut reg = SubsystemRegistry::new();
        reg.register(Box::new(TestSubsystem {
            id: "test.foo",
            state: "hello world".into(),
        }));

        let result = reg.query("test.foo", &Value::Null).unwrap();
        assert!(result.contains("hello world"));
    }

    #[test]
    fn query_unknown_returns_error() {
        let reg = SubsystemRegistry::new();
        let err = reg.query("nonexistent", &Value::Null).unwrap_err();
        assert_eq!(err.code, -32051);
        assert!(err.message.contains("nonexistent"));
    }

    #[test]
    fn list_ids() {
        let mut reg = SubsystemRegistry::new();
        reg.register(Box::new(TestSubsystem {
            id: "a",
            state: "".into(),
        }));
        reg.register(Box::new(TestSubsystem {
            id: "b",
            state: "".into(),
        }));

        let ids = reg.list_ids();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&"a"));
        assert!(ids.contains(&"b"));
    }
}
