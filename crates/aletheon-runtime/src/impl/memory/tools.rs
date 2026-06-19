use async_trait::async_trait;
use serde_json::json;

use aletheon_abi::tool::{PermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta};
use std::sync::Arc;
use tokio::sync::Mutex;

use super::core_memory::CoreMemory;
use super::recall_memory::RecallMemory;

/// Tool: core_memory_append -- append content to a Core Memory block.
pub struct CoreMemoryAppendTool {
    pub memory: Arc<Mutex<CoreMemory>>,
}

#[async_trait]
impl Tool for CoreMemoryAppendTool {
    fn name(&self) -> &str {
        "core_memory_append"
    }
    fn description(&self) -> &str {
        "Append content to a Core Memory block"
    }
    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "label": { "type": "string", "description": "Block label" },
                "content": { "type": "string", "description": "Content to append" }
            },
            "required": ["label", "content"]
        })
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L0
    }
    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(CoreMemoryAppendTool {
            memory: self.memory.clone(),
        })
    }
    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let label = input["label"].as_str().unwrap_or("");
        let content = input["content"].as_str().unwrap_or("");
        let start = std::time::Instant::now();
        let mut mem = self.memory.lock().await;
        match mem.append(label, content) {
            Ok(_) => ToolResult {
                content: format!("Appended to '{}'", label),
                is_error: false,
                metadata: ToolResultMeta {
                    execution_time_ms: start.elapsed().as_millis() as u64,
                    truncated: false,
                },
            },
            Err(e) => ToolResult {
                content: format!("Error: {}", e),
                is_error: true,
                metadata: ToolResultMeta {
                    execution_time_ms: start.elapsed().as_millis() as u64,
                    truncated: false,
                },
            },
        }
    }
}

/// Tool: core_memory_replace -- replace content in a Core Memory block.
pub struct CoreMemoryReplaceTool {
    pub memory: Arc<Mutex<CoreMemory>>,
}

#[async_trait]
impl Tool for CoreMemoryReplaceTool {
    fn name(&self) -> &str {
        "core_memory_replace"
    }
    fn description(&self) -> &str {
        "Replace content in a Core Memory block"
    }
    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "label": { "type": "string", "description": "Block label" },
                "old": { "type": "string", "description": "Content to replace" },
                "new": { "type": "string", "description": "New content" }
            },
            "required": ["label", "old", "new"]
        })
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L0
    }
    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(CoreMemoryReplaceTool {
            memory: self.memory.clone(),
        })
    }
    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let label = input["label"].as_str().unwrap_or("");
        let old = input["old"].as_str().unwrap_or("");
        let new = input["new"].as_str().unwrap_or("");
        let start = std::time::Instant::now();
        let mut mem = self.memory.lock().await;
        match mem.replace(label, old, new) {
            Ok(_) => ToolResult {
                content: format!("Replaced in '{}'", label),
                is_error: false,
                metadata: ToolResultMeta {
                    execution_time_ms: start.elapsed().as_millis() as u64,
                    truncated: false,
                },
            },
            Err(e) => ToolResult {
                content: format!("Error: {}", e),
                is_error: true,
                metadata: ToolResultMeta {
                    execution_time_ms: start.elapsed().as_millis() as u64,
                    truncated: false,
                },
            },
        }
    }
}

/// Tool: memory_search -- search Recall Memory.
pub struct MemorySearchTool {
    pub recall: Arc<Mutex<RecallMemory>>,
}

#[async_trait]
impl Tool for MemorySearchTool {
    fn name(&self) -> &str {
        "memory_search"
    }
    fn description(&self) -> &str {
        "Search conversation history and memory"
    }
    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Search query" },
                "limit": { "type": "integer", "description": "Max results (default 10)" }
            },
            "required": ["query"]
        })
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L0
    }
    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(MemorySearchTool {
            recall: self.recall.clone(),
        })
    }
    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let query = input["query"].as_str().unwrap_or("");
        let limit = input["limit"].as_u64().unwrap_or(10) as usize;
        let start = std::time::Instant::now();
        let recall = self.recall.lock().await;
        match recall.search(query, limit) {
            Ok(entries) => {
                if entries.is_empty() {
                    ToolResult {
                        content: "No results found.".to_string(),
                        is_error: false,
                        metadata: ToolResultMeta {
                            execution_time_ms: start.elapsed().as_millis() as u64,
                            truncated: false,
                        },
                    }
                } else {
                    let lines: Vec<String> = entries
                        .iter()
                        .map(|e| {
                            format!(
                                "[{}] {}: {}",
                                e.timestamp.format("%Y-%m-%d %H:%M"),
                                e.entry_type,
                                if e.content.len() > 200 {
                                    format!("{}...", &e.content[..200])
                                } else {
                                    e.content.clone()
                                }
                            )
                        })
                        .collect();
                    ToolResult {
                        content: lines.join("\n"),
                        is_error: false,
                        metadata: ToolResultMeta {
                            execution_time_ms: start.elapsed().as_millis() as u64,
                            truncated: entries.len() >= limit,
                        },
                    }
                }
            }
            Err(e) => ToolResult {
                content: format!("Search error: {}", e),
                is_error: true,
                metadata: ToolResultMeta {
                    execution_time_ms: start.elapsed().as_millis() as u64,
                    truncated: false,
                },
            },
        }
    }
}
