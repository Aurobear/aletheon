//! Per-leaf configuration provenance and secret-safe rendering.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

/// Ordered application configuration sources, from lowest to highest precedence.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigSourceKind {
    Default,
    System,
    User,
    Project,
    Environment,
    Cli,
}

/// A stable source locator attached to every effective configuration leaf.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigSource {
    pub kind: ConfigSourceKind,
    pub locator: String,
}

impl ConfigSource {
    pub fn new(kind: ConfigSourceKind, locator: impl Into<String>) -> Self {
        Self {
            kind,
            locator: locator.into(),
        }
    }

    pub fn defaults() -> Self {
        Self::new(ConfigSourceKind::Default, "compiled defaults")
    }
}

/// A typed value coupled to its source without exposing the value in Debug output.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenanced<T> {
    pub value: T,
    pub source: ConfigSource,
}

impl<T> fmt::Debug for Provenanced<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Provenanced")
            .field("value", &"<redacted>")
            .field("source", &self.source)
            .finish()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigProvenance {
    leaves: BTreeMap<String, ConfigSource>,
}

impl ConfigProvenance {
    pub(crate) fn record(&mut self, path: String, source: ConfigSource) {
        self.leaves.insert(path, source);
    }

    pub fn source(&self, path: &str) -> Option<&ConfigSource> {
        self.leaves.get(path)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &ConfigSource)> {
        self.leaves
            .iter()
            .map(|(path, source)| (path.as_str(), source))
    }
}

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
                    *value = serde_json::Value::String("<redacted>".into());
                } else {
                    redact_json(value);
                }
            }
        }
        serde_json::Value::Array(values) => values.iter_mut().for_each(redact_json),
        _ => {}
    }
}
