use async_trait::async_trait;
use serde_json::json;
use std::sync::{Arc, RwLock};

use super::super::{PermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta};
use super::BM25Catalog;

/// Bridge tool that implements `Tool` to search deferred tools.
///
/// When the model calls `tool_search`, this tool queries the BM25 catalog
/// and returns matching tool names and descriptions, enabling the model
/// to discover tools that are not in its default tool list.
pub struct ToolSearchTool {
    catalog: Arc<RwLock<BM25Catalog>>,
}

impl ToolSearchTool {
    pub fn new(catalog: Arc<RwLock<BM25Catalog>>) -> Self {
        Self { catalog }
    }
}

#[async_trait]
impl Tool for ToolSearchTool {
    fn name(&self) -> &str {
        "tool_search"
    }

    fn description(&self) -> &str {
        "Search for available tools by description. Returns tool names and descriptions matching the query."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Natural language description of the tool you need"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of results (default: 5)",
                    "default": 5
                }
            },
            "required": ["query"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L0
    }

    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(ToolSearchTool {
            catalog: self.catalog.clone(),
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let query = input["query"].as_str().unwrap_or("");
        let limit = input["limit"].as_u64().unwrap_or(5) as usize;

        let catalog = self.catalog.read().expect("tool catalog lock poisoned");
        let results = catalog.search(query, limit);

        let matches = results
            .iter()
            .map(|(name, score)| {
                let desc = catalog.get_description(name).unwrap_or("(no description)");
                json!({
                    "name": name,
                    "score": score,
                    "description": desc,
                })
            })
            .collect::<Vec<_>>();

        ToolResult {
            // Machine-readable names let the Host expand the model-visible
            // schema set without trusting prose parsing. The execution
            // authority still resolves every name against the immutable
            // profile-authorized catalog.
            content: json!({
                "ok": true,
                "query": query,
                "matches": matches,
                "message": if results.is_empty() {
                    "No matching tools found. Try different keywords."
                } else {
                    "Matching authorized tools can be used on the next inference round."
                }
            })
            .to_string(),
            is_error: false,
            metadata: ToolResultMeta::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{tokenize_and_stem, CatalogEntry};
    use super::*;
    use ::contracts::tool::Tool;
    use ::contracts::tool::ToolExposure;

    fn build_test_catalog() -> BM25Catalog {
        let entries = vec![
            CatalogEntry {
                name: "bash_exec".to_string(),
                description: "Execute a bash command and return stdout/stderr".to_string(),
                tokens: tokenize_and_stem("Execute a bash command and return stdout/stderr"),
                exposure: ToolExposure::Direct,
            },
            CatalogEntry {
                name: "file_read".to_string(),
                description: "Read a file from the filesystem".to_string(),
                tokens: tokenize_and_stem("Read a file from the filesystem"),
                exposure: ToolExposure::Deferred,
            },
            CatalogEntry {
                name: "ebpf_compile".to_string(),
                description: "Compile an eBPF program from C source".to_string(),
                tokens: tokenize_and_stem("Compile an eBPF program from C source"),
                exposure: ToolExposure::Deferred,
            },
            CatalogEntry {
                name: "secret_tool".to_string(),
                description: "Internal system tool".to_string(),
                tokens: tokenize_and_stem("Internal system tool"),
                exposure: ToolExposure::Hidden,
            },
        ];
        BM25Catalog::build(entries)
    }

    #[test]
    fn tool_search_name_and_schema() {
        let tool = ToolSearchTool::new(Arc::new(RwLock::new(build_test_catalog())));
        assert_eq!(tool.name(), "tool_search");
        let schema = tool.input_schema();
        assert!(schema["properties"]["query"].is_object());
    }

    #[tokio::test]
    async fn tool_search_finds_deferred() {
        let tool = ToolSearchTool::new(Arc::new(RwLock::new(build_test_catalog())));
        let ctx = ToolContext {
            approval_authority: None,
            agent: None,
            working_dir: std::path::PathBuf::from("/tmp"),
            session_id: "test".to_string(),
            clock: std::sync::Arc::new(kernel::chronos::TestClock::default()),
            turn_event_sender: None,
        };
        let result = tool
            .execute(json!({"query": "read file", "limit": 5}), &ctx)
            .await;
        assert!(!result.is_error);
        let output: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(output["matches"][0]["name"], "file_read");
    }

    #[tokio::test]
    async fn tool_search_excludes_hidden() {
        let tool = ToolSearchTool::new(Arc::new(RwLock::new(build_test_catalog())));
        let ctx = ToolContext {
            approval_authority: None,
            agent: None,
            working_dir: std::path::PathBuf::from("/tmp"),
            session_id: "test".to_string(),
            clock: std::sync::Arc::new(kernel::chronos::TestClock::default()),
            turn_event_sender: None,
        };
        let result = tool
            .execute(json!({"query": "internal system", "limit": 5}), &ctx)
            .await;
        assert!(!result.is_error);
        let output: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert!(output["matches"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["name"] != "secret_tool"));
    }

    #[tokio::test]
    async fn tool_search_no_match() {
        let tool = ToolSearchTool::new(Arc::new(RwLock::new(build_test_catalog())));
        let ctx = ToolContext {
            approval_authority: None,
            agent: None,
            working_dir: std::path::PathBuf::from("/tmp"),
            session_id: "test".to_string(),
            clock: std::sync::Arc::new(kernel::chronos::TestClock::default()),
            turn_event_sender: None,
        };
        let result = tool
            .execute(json!({"query": "zzzznonexistent", "limit": 5}), &ctx)
            .await;
        assert!(!result.is_error);
        let output: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert!(output["matches"].as_array().unwrap().is_empty());
        assert!(output["message"]
            .as_str()
            .unwrap()
            .contains("No matching tools"));
    }
}
