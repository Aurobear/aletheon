//! Blackboard — schema-flexible JSON key-value area for hypotheses, evidence,
//! and intermediate conclusions (RFC-014).
//!
//! This boundary intentionally does not validate or interpret value schemas.
//! Callers that require typed contracts must validate before writing or after
//! reading; Blackboard only owns top-level key storage and deterministic JSON
//! projection.

use std::collections::HashMap;

use serde_json::Value;

/// A schema-flexible JSON key-value shared workspace area.
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

    /// Merge a JSON object patch at top-level keys.
    ///
    /// Values, including nested objects and arrays, are stored verbatim rather
    /// than recursively merged or schema-validated. Non-object patches are a
    /// deliberate no-op for compatibility.
    pub fn merge(&mut self, patch: Value) {
        if let Value::Object(map) = patch {
            for (k, v) in map {
                self.entries.insert(k, v);
            }
        }
    }

    /// Project all entries to a deterministically ordered JSON object.
    pub fn to_json(&self) -> Value {
        let mut entries = self.entries.iter().collect::<Vec<_>>();
        entries.sort_by(|(left, _), (right, _)| left.cmp(right));
        Value::Object(
            entries
                .into_iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        )
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
    fn merge_non_object_is_a_noop_for_every_json_shape() {
        for patch in [
            json!(null),
            json!(true),
            json!(7),
            json!("text"),
            json!([1, 2]),
        ] {
            let mut bb = Blackboard::new();
            bb.set("kept", json!({"nested": [1, 2, 3]}));
            let before = bb.to_json();

            bb.merge(patch);

            assert_eq!(bb.to_json(), before);
        }
    }

    #[test]
    fn merge_replaces_top_level_value_without_interpreting_nested_schema() {
        let mut bb = Blackboard::new();
        bb.set("record", json!({"old": true, "nested": {"left": 1}}));

        bb.merge(json!({"record": {"nested": {"right": 2}}}));

        assert_eq!(bb.get("record"), Some(&json!({"nested": {"right": 2}})));
    }

    #[test]
    fn to_json_roundtrips() {
        let mut bb = Blackboard::new();
        bb.set("x", json!("y"));
        assert_eq!(bb.to_json(), json!({"x": "y"}));
    }

    #[test]
    fn json_projection_is_stable_across_insertion_order() {
        let mut forward = Blackboard::new();
        forward.set("a", json!({"free": [1, "two", null]}));
        forward.set("b", json!(true));
        let mut reverse = Blackboard::new();
        reverse.set("b", json!(true));
        reverse.set("a", json!({"free": [1, "two", null]}));

        assert_eq!(
            serde_json::to_string(&forward.to_json()).unwrap(),
            serde_json::to_string(&reverse.to_json()).unwrap()
        );
    }
}
