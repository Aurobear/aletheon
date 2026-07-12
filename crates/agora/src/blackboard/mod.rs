//! Blackboard — key-value shared area for hypotheses, evidence, and
//! intermediate conclusions. Absorbs observation/artifact/context (RFC-014).

use std::collections::HashMap;

use serde_json::Value;

/// A JSON key-value shared workspace area.
#[derive(Debug, Clone, Default)]
pub struct Blackboard {
    entries: HashMap<String, Value>,
}

impl Blackboard {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Write (or overwrite) a key.
    pub fn set(&mut self, key: impl Into<String>, value: Value) {
        self.entries.insert(key.into(), value);
    }

    /// Read a key.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.entries.get(key)
    }

    /// Remove a key; returns the removed value if present.
    pub fn remove(&mut self, key: &str) -> Option<Value> {
        self.entries.remove(key)
    }

    /// Merge a JSON object patch (top-level keys) into the blackboard.
    /// Non-object patches are ignored.
    pub fn merge(&mut self, patch: Value) {
        if let Value::Object(map) = patch {
            for (k, v) in map {
                self.entries.insert(k, v);
            }
        }
    }

    /// Serialize all entries to a JSON object.
    pub fn to_json(&self) -> Value {
        Value::Object(self.entries.clone().into_iter().collect())
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn set_get_remove() {
        let mut bb = Blackboard::new();
        bb.set("k", json!(1));
        assert_eq!(bb.get("k"), Some(&json!(1)));
        assert_eq!(bb.remove("k"), Some(json!(1)));
        assert!(bb.is_empty());
    }

    #[test]
    fn merge_object_patch() {
        let mut bb = Blackboard::new();
        bb.set("a", json!(1));
        bb.merge(json!({"b": 2, "a": 9}));
        assert_eq!(bb.get("a"), Some(&json!(9)));
        assert_eq!(bb.get("b"), Some(&json!(2)));
    }

    #[test]
    fn to_json_roundtrips() {
        let mut bb = Blackboard::new();
        bb.set("x", json!("y"));
        assert_eq!(bb.to_json(), json!({"x": "y"}));
    }
}
