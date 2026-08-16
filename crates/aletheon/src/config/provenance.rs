//! Configuration provenance types — owned by `contracts::config_provenance`.
//! Provenance rendering helpers remain binary-owned (application logic).

pub use ::contracts::{ConfigProvenance, ConfigSource, ConfigSourceKind, Provenanced};

pub(crate) fn record_leaves(
    value: &toml::Value,
    prefix: &str,
    source: &ConfigSource,
    provenance: &mut ConfigProvenance,
) {
    match value {
        toml::Value::Table(table) => {
            for (key, value) in table {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                record_leaves(value, &path, source, provenance);
            }
        }
        toml::Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                record_leaves(value, &format!("{prefix}.{index}"), source, provenance);
            }
            if values.is_empty() {
                provenance.record(prefix.to_string(), source.clone());
            }
        }
        _ => provenance.record(prefix.to_string(), source.clone()),
    }
}

pub(crate) fn redact_json(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(object) => {
            for (key, value) in object {
                let normalized = key.to_ascii_lowercase();
                if normalized.contains("secret")
                    || normalized.contains("password")
                    || normalized == "api_key"
                    || normalized.ends_with("_token")
                {
                    // Preserve absence as `null`; replacing an absent optional
                    // credential with a string makes diagnostics claim that a
                    // credential is configured. Every present value, including
                    // a SecretRef object, remains fully redacted.
                    if !value.is_null() {
                        *value = serde_json::Value::String("<redacted>".into());
                    }
                } else {
                    redact_json(value);
                }
            }
        }
        serde_json::Value::Array(values) => values.iter_mut().for_each(redact_json),
        _ => {}
    }
}
