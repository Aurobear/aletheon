use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A directed edge in the workflow graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    /// Source node ID.
    pub from: String,
    /// Target node ID.
    pub to: String,
    /// Condition for traversing this edge.
    pub condition: ConditionExpr,
}

/// Condition expression for edge traversal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConditionExpr {
    /// Always traverse.
    Always,
    /// Traverse if a key equals a value.
    Equals(String, serde_json::Value),
    /// Traverse if a key exists.
    Exists(String),
    /// Traverse if a key is truthy.
    IsTruthy(String),
}

impl ConditionExpr {
    /// Evaluate this condition against the given data.
    pub fn evaluate(&self, data: &HashMap<String, serde_json::Value>) -> bool {
        match self {
            ConditionExpr::Always => true,
            ConditionExpr::Equals(key, expected) => {
                data.get(key).map_or(false, |v| v == expected)
            }
            ConditionExpr::Exists(key) => data.contains_key(key),
            ConditionExpr::IsTruthy(key) => {
                data.get(key).map_or(false, |v| match v {
                    serde_json::Value::Bool(b) => *b,
                    serde_json::Value::Number(n) => n.as_f64().map_or(false, |f| f != 0.0),
                    serde_json::Value::String(s) => !s.is_empty(),
                    serde_json::Value::Array(a) => !a.is_empty(),
                    serde_json::Value::Object(o) => !o.is_empty(),
                    serde_json::Value::Null => false,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_always() {
        let data = HashMap::new();
        assert!(ConditionExpr::Always.evaluate(&data));
    }

    #[test]
    fn test_equals() {
        let mut data = HashMap::new();
        data.insert("status".to_string(), serde_json::json!("ok"));
        let cond = ConditionExpr::Equals("status".into(), serde_json::json!("ok"));
        assert!(cond.evaluate(&data));
        let cond2 = ConditionExpr::Equals("status".into(), serde_json::json!("fail"));
        assert!(!cond2.evaluate(&data));
    }

    #[test]
    fn test_exists() {
        let mut data = HashMap::new();
        data.insert("key".to_string(), serde_json::json!(null));
        assert!(ConditionExpr::Exists("key".into()).evaluate(&data));
        assert!(!ConditionExpr::Exists("missing".into()).evaluate(&data));
    }

    #[test]
    fn test_is_truthy() {
        let mut data = HashMap::new();
        data.insert("a".to_string(), serde_json::json!(true));
        data.insert("b".to_string(), serde_json::json!(false));
        data.insert("c".to_string(), serde_json::json!("hello"));
        data.insert("d".to_string(), serde_json::json!(""));
        data.insert("e".to_string(), serde_json::json!(42));
        data.insert("f".to_string(), serde_json::json!(0));
        data.insert("g".to_string(), serde_json::json!([]));
        data.insert("h".to_string(), serde_json::json!([1]));
        data.insert("i".to_string(), serde_json::json!(null));

        assert!(ConditionExpr::IsTruthy("a".into()).evaluate(&data));
        assert!(!ConditionExpr::IsTruthy("b".into()).evaluate(&data));
        assert!(ConditionExpr::IsTruthy("c".into()).evaluate(&data));
        assert!(!ConditionExpr::IsTruthy("d".into()).evaluate(&data));
        assert!(ConditionExpr::IsTruthy("e".into()).evaluate(&data));
        assert!(!ConditionExpr::IsTruthy("f".into()).evaluate(&data));
        assert!(!ConditionExpr::IsTruthy("g".into()).evaluate(&data));
        assert!(ConditionExpr::IsTruthy("h".into()).evaluate(&data));
        assert!(!ConditionExpr::IsTruthy("i".into()).evaluate(&data));
        assert!(!ConditionExpr::IsTruthy("missing".into()).evaluate(&data));
    }
}
