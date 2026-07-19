use async_trait::async_trait;
use serde_json::json;

use super::super::{PermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta};
use super::BM25Catalog;

/// Bridge tool that implements `Tool` to search deferred tools.
///
/// When the model calls `tool_search`, this tool queries the BM25 catalog
/// and returns matching tool names and descriptions, enabling the model
/// to discover tools that are not in its default tool list.
pub struct ToolSearchTool {
    catalog: BM25Catalog,
}

impl ToolSearchTool {
    pub fn new(catalog: BM25Catalog) -> Self {
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
        // ToolSearchTool owns a catalog; boxed_clone clones the catalog.
        Box::new(ToolSearchTool {
            catalog: BM25Catalog::build(self.catalog.entries_clone()),
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let query = input["query"].as_str().unwrap_or("");
        let limit = input["limit"].as_u64().unwrap_or(5) as usize;

        let results = self.catalog.search(query, limit);

        if results.is_empty() {
            return ToolResult {
                content: "No matching tools found. Try different keywords.".to_string(),
                is_error: false,
                metadata: ToolResultMeta::default(),
            };
        }

        let lines: Vec<String> = results
            .iter()
            .map(|(name, score)| {
                let desc = self
                    .catalog
                    .get_description(name)
                    .unwrap_or("(no description)");
                format!("- {} (score: {:.2}): {}", name, score, desc)
            })
            .collect();

        ToolResult {
            content: format!("Found {} tool(s):\n{}", lines.len(), lines.join("\n")),
            is_error: false,
            metadata: ToolResultMeta::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{tokenize_and_stem, CatalogEntry};
    use super::*;
    use fabric::tool::Tool;
    use fabric::tool::ToolExposure;

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
        let tool = ToolSearchTool::new(build_test_catalog());
        assert_eq!(tool.name(), "tool_search");
        let schema = tool.input_schema();
        assert!(schema["properties"]["query"].is_object());
    }

    #[tokio::test]
    async fn tool_search_finds_deferred() {
        let tool = ToolSearchTool::new(build_test_catalog());
        let ctx = ToolContext {
            approval_authority: None,
            agent: None,
            working_dir: std::path::PathBuf::from("/tmp"),
            session_id: "test".to_string(),
            clock: std::sync::Arc::new(aletheon_kernel::chronos::TestClock::default()),
            turn_event_sender: None,
        };
        let result = tool
            .execute(json!({"query": "read file", "limit": 5}), &ctx)
            .await;
        assert!(!result.is_error);
        assert!(result.content.contains("file_read"));
    }

    #[tokio::test]
    async fn tool_search_excludes_hidden() {
        let tool = ToolSearchTool::new(build_test_catalog());
        let ctx = ToolContext {
            approval_authority: None,
            agent: None,
            working_dir: std::path::PathBuf::from("/tmp"),
            session_id: "test".to_string(),
            clock: std::sync::Arc::new(aletheon_kernel::chronos::TestClock::default()),
            turn_event_sender: None,
        };
        let result = tool
            .execute(json!({"query": "internal system", "limit": 5}), &ctx)
            .await;
        assert!(!result.is_error);
        assert!(!result.content.contains("secret_tool"));
    }

    #[tokio::test]
    async fn tool_search_no_match() {
        let tool = ToolSearchTool::new(build_test_catalog());
        let ctx = ToolContext {
            approval_authority: None,
            agent: None,
            working_dir: std::path::PathBuf::from("/tmp"),
            session_id: "test".to_string(),
            clock: std::sync::Arc::new(aletheon_kernel::chronos::TestClock::default()),
            turn_event_sender: None,
        };
        let result = tool
            .execute(json!({"query": "zzzznonexistent", "limit": 5}), &ctx)
            .await;
        assert!(!result.is_error);
        assert!(result.content.contains("No matching tools"));
    }
}
