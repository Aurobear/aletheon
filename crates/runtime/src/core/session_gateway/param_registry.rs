//! Dynamic parameter registry with lazy evaluation.
//!
//! Each parameter is a key + a getter closure that produces a JSON value.
//! Getters are called on each query (no caching), ensuring live values.
//! Replaces the 8 hard-coded values previously in `debug_handler.rs`.

use serde_json::Value;
use std::collections::HashMap;
use tokio::sync::RwLock;

/// Metadata for a registered parameter.
#[derive(Clone)]
pub struct ParamInfo {
    pub key: String,
    pub namespace: String,
    pub description: String,
}

struct ParamEntry {
    info: ParamInfo,
    /// Getter: called each time the param is read.
    getter: Box<dyn Fn() -> Value + Send + Sync>,
}

/// Dynamic parameter registry — the ROS-style `rosparam` equivalent.
///
/// Subsystems register their live state as named parameters at init time.
/// The Session Gateway reads them on demand via `get()` / `list()`.
pub struct ParamRegistry {
    params: RwLock<HashMap<String, ParamEntry>>,
}

impl Default for ParamRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ParamRegistry {
    pub fn new() -> Self {
        Self {
            params: RwLock::new(HashMap::new()),
        }
    }

    /// Register a parameter.
    ///
    /// The getter is called each time the param is read, so values are always live.
    ///
    /// # Example
    /// ```ignore
    /// registry.declare(
    ///     "react.tool_calls_remaining",
    ///     "react",
    ///     "Number of tool calls remaining in current turn budget",
    ///     || json!(3),
    /// );
    /// ```
    pub async fn declare(
        &self,
        key: &str,
        namespace: &str,
        description: &str,
        getter: impl Fn() -> Value + Send + Sync + 'static,
    ) {
        let entry = ParamEntry {
            info: ParamInfo {
                key: key.to_string(),
                namespace: namespace.to_string(),
                description: description.to_string(),
            },
            getter: Box::new(getter),
        };
        self.params.write().await.insert(key.to_string(), entry);
    }

    /// Get a single parameter value.
    ///
    /// Returns `None` if the key is not registered.
    pub async fn get(&self, key: &str) -> Option<Value> {
        let params = self.params.read().await;
        params.get(key).map(|entry| (entry.getter)())
    }

    /// List all parameters, optionally filtered by namespace.
    ///
    /// Returns a map of key → value for matching params.
    pub async fn list(&self, namespace: Option<&str>) -> HashMap<String, Value> {
        let params = self.params.read().await;
        params
            .iter()
            .filter(|(_, entry)| {
                namespace
                    .map(|ns| entry.info.namespace == ns)
                    .unwrap_or(true)
            })
            .map(|(key, entry)| (key.clone(), (entry.getter)()))
            .collect()
    }

    /// Dump all parameters with their descriptions.
    pub async fn dump(&self) -> Vec<ParamInfo> {
        let params = self.params.read().await;
        params.values().map(|entry| entry.info.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn declare_and_get() {
        let reg = ParamRegistry::new();
        reg.declare("test.value", "test", "A test param", || json!(42))
            .await;

        let val = reg.get("test.value").await;
        assert_eq!(val, Some(json!(42)));
    }

    #[tokio::test]
    async fn get_unknown_returns_none() {
        let reg = ParamRegistry::new();
        assert_eq!(reg.get("nonexistent").await, None);
    }

    #[tokio::test]
    async fn list_all_and_filtered() {
        let reg = ParamRegistry::new();
        reg.declare("a.x", "a", "param in namespace a", || json!(1))
            .await;
        reg.declare("a.y", "a", "another a param", || json!(2))
            .await;
        reg.declare("b.z", "b", "param in namespace b", || json!(3))
            .await;

        let all = reg.list(None).await;
        assert_eq!(all.len(), 3);
        assert_eq!(all["a.x"], json!(1));
        assert_eq!(all["a.y"], json!(2));
        assert_eq!(all["b.z"], json!(3));

        let a_only = reg.list(Some("a")).await;
        assert_eq!(a_only.len(), 2);
        assert!(a_only.contains_key("a.x"));
        assert!(a_only.contains_key("a.y"));

        let b_only = reg.list(Some("b")).await;
        assert_eq!(b_only.len(), 1);
        assert_eq!(b_only["b.z"], json!(3));

        let none = reg.list(Some("c")).await;
        assert_eq!(none.len(), 0);
    }

    #[tokio::test]
    async fn dump_returns_info() {
        let reg = ParamRegistry::new();
        reg.declare("x", "ns", "desc x", || json!(1)).await;
        reg.declare("y", "ns", "desc y", || json!(2)).await;

        let mut dump = reg.dump().await;
        dump.sort_by(|a, b| a.key.cmp(&b.key));

        assert_eq!(dump.len(), 2);
        assert_eq!(dump[0].key, "x");
        assert_eq!(dump[0].description, "desc x");
        assert_eq!(dump[1].key, "y");
        assert_eq!(dump[1].description, "desc y");
    }

    #[tokio::test]
    async fn getter_evaluated_on_each_call() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let counter = Arc::new(AtomicUsize::new(0));
        let c = counter.clone();
        let reg = ParamRegistry::new();
        reg.declare("counter", "test", "incrementing counter", move || {
            json!(c.fetch_add(1, Ordering::SeqCst))
        })
        .await;

        assert_eq!(reg.get("counter").await, Some(json!(0)));
        assert_eq!(reg.get("counter").await, Some(json!(1)));
        assert_eq!(reg.get("counter").await, Some(json!(2)));
    }
}
