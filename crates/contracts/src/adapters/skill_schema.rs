//! Bounded JSON Schema validation for robot skill input contracts.
//!
//! The supported dialect is deliberately explicit. Unknown assertion keywords
//! fail closed, preventing a descriptor from looking validated while silently
//! relying on semantics this runtime does not implement.

use std::collections::BTreeSet;

use serde_json::{Map, Value};

const MAX_SCHEMA_DEPTH: usize = 16;
const MAX_SCHEMA_NODES: usize = 512;

pub(crate) fn validate_skill_input_schema(schema: &Value) -> Result<(), String> {
    let object = schema
        .as_object()
        .ok_or_else(|| "$: skill input schema must be an object".to_owned())?;
    if object.get("type").and_then(Value::as_str) != Some("object") {
        return Err("$: skill input schema type must be object".into());
    }
    if object.get("additionalProperties") != Some(&Value::Bool(false)) {
        return Err("$: skill input schema must set additionalProperties=false".into());
    }
    let mut nodes = 0;
    validate_schema_at(schema, "$", 0, &mut nodes)
}

pub(crate) fn validate_instance(schema: &Value, value: &Value) -> Result<(), String> {
    validate_skill_input_schema(schema)?;
    validate_value_at(schema, value, "$", 0)
}

fn validate_schema_at(
    schema: &Value,
    path: &str,
    depth: usize,
    nodes: &mut usize,
) -> Result<(), String> {
    if depth > MAX_SCHEMA_DEPTH {
        return Err(format!("{path}: schema nesting exceeds {MAX_SCHEMA_DEPTH}"));
    }
    *nodes += 1;
    if *nodes > MAX_SCHEMA_NODES {
        return Err(format!("schema node count exceeds {MAX_SCHEMA_NODES}"));
    }
    let object = schema
        .as_object()
        .ok_or_else(|| format!("{path}: schema must be an object"))?;
    reject_unknown_keywords(object, path)?;

    if let Some(kind) = object.get("type") {
        let kind = kind
            .as_str()
            .ok_or_else(|| format!("{path}.type must be a string"))?;
        if !matches!(
            kind,
            "object" | "array" | "string" | "number" | "integer" | "boolean" | "null"
        ) {
            return Err(format!("{path}.type has unsupported value {kind:?}"));
        }
    } else if !object.contains_key("const")
        && !object.contains_key("enum")
        && !object.contains_key("allOf")
        && !object.contains_key("anyOf")
        && !object.contains_key("oneOf")
    {
        return Err(format!(
            "{path}: schema requires type or a composition assertion"
        ));
    }

    let properties = object.get("properties");
    if let Some(properties) = properties {
        let properties = properties
            .as_object()
            .ok_or_else(|| format!("{path}.properties must be an object"))?;
        for (name, child) in properties {
            validate_schema_at(
                child,
                &format!("{path}.properties.{name}"),
                depth + 1,
                nodes,
            )?;
        }
    }
    if let Some(required) = object.get("required") {
        let required = string_array(required, &format!("{path}.required"))?;
        let unique = required.iter().copied().collect::<BTreeSet<_>>();
        if unique.len() != required.len() {
            return Err(format!("{path}.required contains duplicate names"));
        }
        let property_names = properties
            .and_then(Value::as_object)
            .map(|map| map.keys().map(String::as_str).collect::<BTreeSet<_>>())
            .unwrap_or_default();
        for name in required {
            if !property_names.contains(name) {
                return Err(format!("{path}.required names unknown property {name:?}"));
            }
        }
    }
    if let Some(additional) = object.get("additionalProperties") {
        if !additional.is_boolean() {
            validate_schema_at(
                additional,
                &format!("{path}.additionalProperties"),
                depth + 1,
                nodes,
            )?;
        }
    }
    if let Some(items) = object.get("items") {
        validate_schema_at(items, &format!("{path}.items"), depth + 1, nodes)?;
    }
    for keyword in ["allOf", "anyOf", "oneOf"] {
        if let Some(children) = object.get(keyword) {
            let children = children
                .as_array()
                .ok_or_else(|| format!("{path}.{keyword} must be an array"))?;
            if children.is_empty() {
                return Err(format!("{path}.{keyword} must not be empty"));
            }
            for (index, child) in children.iter().enumerate() {
                validate_schema_at(
                    child,
                    &format!("{path}.{keyword}[{index}]"),
                    depth + 1,
                    nodes,
                )?;
            }
        }
    }
    if let Some(values) = object.get("enum") {
        let values = values
            .as_array()
            .ok_or_else(|| format!("{path}.enum must be an array"))?;
        if values.is_empty() {
            return Err(format!("{path}.enum must not be empty"));
        }
    }
    if object
        .get("uniqueItems")
        .is_some_and(|value| !value.is_boolean())
    {
        return Err(format!("{path}.uniqueItems must be a boolean"));
    }
    for keyword in ["minimum", "maximum", "exclusiveMinimum", "exclusiveMaximum"] {
        if object.get(keyword).is_some_and(|value| !value.is_number()) {
            return Err(format!("{path}.{keyword} must be a number"));
        }
    }
    for keyword in ["minLength", "maxLength", "minItems", "maxItems"] {
        if let Some(value) = object.get(keyword) {
            unsigned(value, &format!("{path}.{keyword}"))?;
        }
    }
    if let Some(pattern) = object.get("pattern") {
        let pattern = pattern
            .as_str()
            .ok_or_else(|| format!("{path}.pattern must be a string"))?;
        regex::Regex::new(pattern).map_err(|error| format!("{path}.pattern: {error}"))?;
    }
    Ok(())
}

fn reject_unknown_keywords(object: &Map<String, Value>, path: &str) -> Result<(), String> {
    const SUPPORTED: &[&str] = &[
        "$schema",
        "$id",
        "title",
        "description",
        "default",
        "examples",
        "type",
        "properties",
        "required",
        "additionalProperties",
        "items",
        "enum",
        "const",
        "minimum",
        "maximum",
        "exclusiveMinimum",
        "exclusiveMaximum",
        "minLength",
        "maxLength",
        "pattern",
        "minItems",
        "maxItems",
        "uniqueItems",
        "allOf",
        "anyOf",
        "oneOf",
    ];
    for keyword in object.keys() {
        if !SUPPORTED.contains(&keyword.as_str()) {
            return Err(format!("{path}: unsupported schema keyword {keyword:?}"));
        }
    }
    Ok(())
}

fn validate_value_at(
    schema: &Value,
    value: &Value,
    path: &str,
    depth: usize,
) -> Result<(), String> {
    if depth > MAX_SCHEMA_DEPTH {
        return Err(format!(
            "{path}: instance nesting exceeds {MAX_SCHEMA_DEPTH}"
        ));
    }
    let object = schema.as_object().expect("schema was validated");
    if let Some(expected) = object.get("type").and_then(Value::as_str) {
        let matches = match expected {
            "object" => value.is_object(),
            "array" => value.is_array(),
            "string" => value.is_string(),
            "number" => value.is_number(),
            "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
            "boolean" => value.is_boolean(),
            "null" => value.is_null(),
            _ => false,
        };
        if !matches {
            return Err(format!("{path}: expected {expected}"));
        }
    }
    if let Some(expected) = object.get("const") {
        if value != expected {
            return Err(format!("{path}: value does not match const"));
        }
    }
    if let Some(values) = object.get("enum").and_then(Value::as_array) {
        if !values.contains(value) {
            return Err(format!("{path}: value is outside enum"));
        }
    }
    validate_compositions(object, value, path, depth)?;

    if let Some(map) = value.as_object() {
        let properties = object
            .get("properties")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        if let Some(required) = object.get("required") {
            for name in string_array(required, &format!("{path}.required"))? {
                if !map.contains_key(name) {
                    return Err(format!("{path}: missing required property {name:?}"));
                }
            }
        }
        for (name, child) in map {
            if let Some(child_schema) = properties.get(name) {
                validate_value_at(child_schema, child, &format!("{path}.{name}"), depth + 1)?;
            } else if let Some(additional) = object.get("additionalProperties") {
                if additional == &Value::Bool(false) {
                    return Err(format!("{path}: unexpected property {name:?}"));
                }
                if additional.is_object() {
                    validate_value_at(additional, child, &format!("{path}.{name}"), depth + 1)?;
                }
            }
        }
    }
    if let Some(values) = value.as_array() {
        check_count(object, "minItems", "maxItems", values.len(), path)?;
        if object.get("uniqueItems") == Some(&Value::Bool(true)) {
            for (index, candidate) in values.iter().enumerate() {
                if values[..index].contains(candidate) {
                    return Err(format!("{path}: array items must be unique"));
                }
            }
        }
        if let Some(items) = object.get("items") {
            for (index, child) in values.iter().enumerate() {
                validate_value_at(items, child, &format!("{path}[{index}]"), depth + 1)?;
            }
        }
    }
    if let Some(text) = value.as_str() {
        check_count(object, "minLength", "maxLength", text.chars().count(), path)?;
        if let Some(pattern) = object.get("pattern").and_then(Value::as_str) {
            if !regex::Regex::new(pattern)
                .expect("pattern was validated")
                .is_match(text)
            {
                return Err(format!("{path}: string does not match pattern"));
            }
        }
    }
    if let Some(number) = value.as_f64() {
        check_number(object, "minimum", number, false, path)?;
        check_number(object, "maximum", number, true, path)?;
        check_number(object, "exclusiveMinimum", number, false, path)?;
        check_number(object, "exclusiveMaximum", number, true, path)?;
    }
    Ok(())
}

fn validate_compositions(
    object: &Map<String, Value>,
    value: &Value,
    path: &str,
    depth: usize,
) -> Result<(), String> {
    for keyword in ["allOf", "anyOf", "oneOf"] {
        let Some(children) = object.get(keyword).and_then(Value::as_array) else {
            continue;
        };
        let matches = children
            .iter()
            .filter(|schema| validate_value_at(schema, value, path, depth + 1).is_ok())
            .count();
        let valid = match keyword {
            "allOf" => matches == children.len(),
            "anyOf" => matches >= 1,
            "oneOf" => matches == 1,
            _ => unreachable!(),
        };
        if !valid {
            return Err(format!("{path}: {keyword} matched {matches} schemas"));
        }
    }
    Ok(())
}

fn check_count(
    object: &Map<String, Value>,
    min_key: &str,
    max_key: &str,
    actual: usize,
    path: &str,
) -> Result<(), String> {
    if let Some(minimum) = object.get(min_key) {
        if actual < unsigned(minimum, min_key)? {
            return Err(format!("{path}: length is below {min_key}"));
        }
    }
    if let Some(maximum) = object.get(max_key) {
        if actual > unsigned(maximum, max_key)? {
            return Err(format!("{path}: length exceeds {max_key}"));
        }
    }
    Ok(())
}

fn check_number(
    object: &Map<String, Value>,
    keyword: &str,
    actual: f64,
    upper: bool,
    path: &str,
) -> Result<(), String> {
    let Some(limit) = object.get(keyword).and_then(Value::as_f64) else {
        return Ok(());
    };
    let exclusive = keyword.starts_with("exclusive");
    let invalid = if upper {
        if exclusive {
            actual >= limit
        } else {
            actual > limit
        }
    } else if exclusive {
        actual <= limit
    } else {
        actual < limit
    };
    if invalid {
        return Err(format!("{path}: number violates {keyword}={limit}"));
    }
    Ok(())
}

fn string_array<'a>(value: &'a Value, path: &str) -> Result<Vec<&'a str>, String> {
    value
        .as_array()
        .ok_or_else(|| format!("{path} must be an array"))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .ok_or_else(|| format!("{path} entries must be strings"))
        })
        .collect()
}

fn unsigned(value: &Value, path: &str) -> Result<usize, String> {
    value
        .as_u64()
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| format!("{path} must be a non-negative integer"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounded_object_schema() -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "duration_ms": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 2_000
                },
                "mode": {
                    "type": "string",
                    "enum": ["walk", "stance"]
                }
            },
            "required": ["duration_ms", "mode"],
            "additionalProperties": false
        })
    }

    #[test]
    fn validates_bounded_skill_parameters() {
        let schema = bounded_object_schema();
        validate_instance(
            &schema,
            &serde_json::json!({"duration_ms": 500, "mode": "walk"}),
        )
        .unwrap();
        assert!(validate_instance(
            &schema,
            &serde_json::json!({"duration_ms": 0, "mode": "walk"})
        )
        .is_err());
        assert!(validate_instance(
            &schema,
            &serde_json::json!({"duration_ms": 500, "mode": "run"})
        )
        .is_err());
        assert!(validate_instance(
            &schema,
            &serde_json::json!({"duration_ms": 500, "mode": "walk", "joint": 1})
        )
        .is_err());
    }

    #[test]
    fn rejects_open_or_unsupported_skill_schemas() {
        let mut open = bounded_object_schema();
        open["additionalProperties"] = Value::Bool(true);
        assert!(validate_skill_input_schema(&open).is_err());

        let mut unsupported = bounded_object_schema();
        unsupported["$ref"] = Value::String("#/$defs/input".into());
        assert!(validate_skill_input_schema(&unsupported).is_err());

        let mut duplicate_required = bounded_object_schema();
        duplicate_required["required"] = serde_json::json!(["mode", "mode"]);
        assert!(validate_skill_input_schema(&duplicate_required).is_err());
    }
}
