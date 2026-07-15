//! Pinned GBrain HTTP MCP contract validation and transport-neutral settings.

use std::collections::BTreeSet;

use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PINNED_RELEASE: &str = "v0.42.59.0";
pub const PINNED_COMMIT: &str = "5008b287e47bf791132eedfebf66bdef11e9398c";
pub const REQUIRED_TOOLS: [&str; 4] = ["query", "search", "get_page", "put_page"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetryPolicy {
    pub initial_delay_ms: u64,
    pub max_delay_ms: u64,
    pub max_attempts: u32,
    pub max_age_secs: u64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            initial_delay_ms: 1_000,
            max_delay_ms: 60_000,
            max_attempts: 12,
            max_age_secs: 86_400,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpoolPolicy {
    pub path: String,
    pub max_items: usize,
    pub max_bytes: u64,
    pub legacy_outbox_dir: Option<String>,
}

impl Default for SpoolPolicy {
    fn default() -> Self {
        Self {
            path: "~/.aletheon/memory/gbrain-spool.db".into(),
            max_items: 10_000,
            max_bytes: 256 * 1024 * 1024,
            legacy_outbox_dir: Some("~/.aletheon/gbrain-outbox".into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GbrainBackendConfig {
    pub enabled: bool,
    pub server_name: String,
    pub read_sources: Vec<String>,
    pub write_source: String,
    pub request_timeout_ms: u64,
    pub delivery_batch_size: usize,
    pub recall_limit: usize,
    pub schema_fixture: String,
    pub schema_version: String,
    pub retry: RetryPolicy,
    pub spool: SpoolPolicy,
}

impl Default for GbrainBackendConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            server_name: "gbrain".into(),
            read_sources: vec!["aletheon".into(), "general".into()],
            write_source: "aletheon".into(),
            request_timeout_ms: 1_200,
            delivery_batch_size: 20,
            recall_limit: 4,
            schema_fixture: "config/gbrain/tools-schema.json".into(),
            schema_version: PINNED_RELEASE.into(),
            retry: RetryPolicy::default(),
            spool: SpoolPolicy::default(),
        }
    }
}

pub fn validate_tools_list(document: &Value) -> anyhow::Result<()> {
    let tools = document
        .pointer("/result/tools")
        .and_then(Value::as_array)
        .context("GBrain tools/list response lacks result.tools")?;
    for (name, required, properties) in [
        ("query", &[][..], &["query", "source_id", "limit"][..]),
        ("search", &["query"][..], &["query", "limit"][..]),
        ("get_page", &["slug"][..], &["slug"][..]),
        (
            "put_page",
            &["slug", "content"][..],
            &["slug", "content"][..],
        ),
    ] {
        let tool = tools
            .iter()
            .find(|tool| tool.get("name").and_then(Value::as_str) == Some(name))
            .with_context(|| format!("GBrain required MCP tool `{name}` is missing"))?;
        let schema = tool
            .get("inputSchema")
            .context("required tool lacks inputSchema")?;
        if schema.get("type").and_then(Value::as_str) != Some("object") {
            bail!("GBrain tool `{name}` inputSchema is not an object");
        }
        let props = schema
            .get("properties")
            .and_then(Value::as_object)
            .with_context(|| format!("GBrain tool `{name}` lacks properties"))?;
        for property in properties {
            let value = props
                .get(*property)
                .with_context(|| format!("GBrain tool `{name}` lacks `{property}`"))?;
            let expected = if *property == "limit" {
                "number"
            } else {
                "string"
            };
            if value.get("type").and_then(Value::as_str) != Some(expected) {
                bail!("GBrain tool `{name}` property `{property}` has incompatible type");
            }
        }
        let actual: BTreeSet<_> = schema
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        if required.iter().any(|field| !actual.contains(field)) {
            bail!("GBrain tool `{name}` has incompatible required fields");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_fixture_has_required_compatible_tools() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../../../config/gbrain/tools-schema.json"
        ))
        .unwrap();
        validate_tools_list(&fixture).unwrap();
    }

    #[test]
    fn rejects_missing_or_incompatible_required_tools() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../../../config/gbrain/tools-schema.json"
        ))
        .unwrap();
        let mut missing = fixture.clone();
        missing
            .pointer_mut("/result/tools")
            .unwrap()
            .as_array_mut()
            .unwrap()
            .retain(|tool| tool["name"] != "put_page");
        assert!(validate_tools_list(&missing)
            .unwrap_err()
            .to_string()
            .contains("put_page"));
        let mut wrong = fixture;
        let query = wrong
            .pointer_mut("/result/tools")
            .unwrap()
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|tool| tool["name"] == "query")
            .unwrap();
        query["inputSchema"]["properties"]["source_id"]["type"] = Value::String("number".into());
        assert!(validate_tools_list(&wrong).is_err());
    }
}
