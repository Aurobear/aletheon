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
    pub fn record(&mut self, path: String, source: ConfigSource) {
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
