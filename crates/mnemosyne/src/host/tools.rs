use async_trait::async_trait;
use serde_json::json;

use fabric::tool::{PermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta};
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::adapters::storage::fact_store::FactStore;
use crate::adapters::storage::recall_memory::RecallMemory;
use crate::domain::core_memory::CoreMemory;

/// Tool: core_memory_append -- append content to a Core Memory block.
pub struct CoreMemoryAppendTool {
    pub memory: Arc<Mutex<CoreMemory>>,
    pub clock: Arc<dyn fabric::Clock>,
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
            clock: self.clock.clone(),
        })
    }
    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let label = input["label"].as_str().unwrap_or("");
        let content = input["content"].as_str().unwrap_or("");
        let start = self.clock.mono_now().0;
        let mut mem = self.memory.lock().await;
        let elapsed = self.clock.mono_now().0.saturating_sub(start);
        match mem.append(label, content) {
            Ok(_) => ToolResult {
                content: format!("Appended to '{label}'"),
                is_error: false,
                metadata: ToolResultMeta {
                    execution_time_ms: elapsed,
                    truncated: false,
                    patch_delta: None,
                },
            },
            Err(e) => ToolResult {
                content: format!("Error: {e}"),
                is_error: true,
                metadata: ToolResultMeta {
                    execution_time_ms: elapsed,
                    truncated: false,
                    patch_delta: None,
                },
            },
        }
    }
}

/// Tool: core_memory_replace -- replace content in a Core Memory block.
pub struct CoreMemoryReplaceTool {
    pub memory: Arc<Mutex<CoreMemory>>,
    pub clock: Arc<dyn fabric::Clock>,
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
            clock: self.clock.clone(),
        })
    }
    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let label = input["label"].as_str().unwrap_or("");
        let old = input["old"].as_str().unwrap_or("");
        let new = input["new"].as_str().unwrap_or("");
        let start = self.clock.mono_now().0;
        let mut mem = self.memory.lock().await;
        let elapsed = self.clock.mono_now().0.saturating_sub(start);
        match mem.replace(label, old, new) {
            Ok(_) => ToolResult {
                content: format!("Replaced in '{label}'"),
                is_error: false,
                metadata: ToolResultMeta {
                    execution_time_ms: elapsed,
                    truncated: false,
                    patch_delta: None,
                },
            },
            Err(e) => ToolResult {
                content: format!("Error: {e}"),
                is_error: true,
                metadata: ToolResultMeta {
                    execution_time_ms: elapsed,
                    truncated: false,
                    patch_delta: None,
                },
            },
        }
    }
}

/// Tool: memory_search -- unified search across CoreMemory, FactStore, and RecallMemory.
pub struct MemorySearchTool {
    pub recall: Arc<Mutex<RecallMemory>>,
    pub core_memory: Arc<Mutex<CoreMemory>>,
    pub fact_store: Option<Arc<Mutex<FactStore>>>,
    pub clock: Arc<dyn fabric::Clock>,
}

impl MemorySearchTool {
    /// Search CoreMemory blocks by case-insensitive substring matching.
    fn search_core_memory(core: &CoreMemory, query_lower: &str) -> Vec<String> {
        let mut results = Vec::new();
        for (label, block) in core.blocks() {
            if block.value.is_empty() {
                continue;
            }
            // Split block value into lines and match per-line
            for line in block.value.lines() {
                if line.to_lowercase().contains(query_lower) {
                    let preview = if line.len() > 200 {
                        let end = line
                            .char_indices()
                            .nth(200)
                            .map(|(i, _)| i)
                            .unwrap_or(line.len());
                        format!("{}...", &line[..end])
                    } else {
                        line.to_string()
                    };
                    results.push(format!("[core:{label}] {preview}"));
                }
            }
        }
        results
    }

    /// Search FactStore by FTS5 query, formatted with trust scores.
    fn search_fact_store(fact_store: &FactStore, query: &str, limit: usize) -> Vec<String> {
        let mut results = Vec::new();
        match fact_store.search_facts(query, None, 0.1, limit) {
            Ok(facts) => {
                for fact in facts {
                    let preview = if fact.content.len() > 200 {
                        let end = fact
                            .content
                            .char_indices()
                            .nth(200)
                            .map(|(i, _)| i)
                            .unwrap_or(fact.content.len());
                        format!("{}...", &fact.content[..end])
                    } else {
                        fact.content.clone()
                    };
                    results.push(format!(
                        "[fact:{}] {} (trust: {:.2}, category: {})",
                        fact.fact_id, preview, fact.trust_score, fact.category
                    ));
                }
            }
            Err(e) => {
                results.push(format!("[fact:search error] {e}"));
            }
        }
        results
    }
}

#[async_trait]
impl Tool for MemorySearchTool {
    fn name(&self) -> &str {
        "memory_search"
    }
    fn description(&self) -> &str {
        "Search across all memory stores: core memory blocks, facts, and conversation history"
    }
    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Search query" },
                "limit": { "type": "integer", "description": "Max results per store (default 5)" }
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
            core_memory: self.core_memory.clone(),
            fact_store: self.fact_store.clone(),
            clock: self.clock.clone(),
        })
    }
    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let query = input["query"].as_str().unwrap_or("");
        let limit = input["limit"].as_u64().unwrap_or(5) as usize;
        let start = self.clock.mono_now().0;
        let query_lower = query.to_lowercase();
        let mut all_results: Vec<String> = Vec::new();

        // 1. Search CoreMemory (in-memory substring match)
        {
            let core = self.core_memory.lock().await;
            let core_results = Self::search_core_memory(&core, &query_lower);
            all_results.extend(core_results);
        }

        // 2. Search FactStore (SQLite FTS5, if available)
        if let Some(ref fs) = self.fact_store {
            let fact_store = fs.lock().await;
            let fact_results = Self::search_fact_store(&fact_store, query, limit);
            all_results.extend(fact_results);
        }

        // 3. Search RecallMemory (SQLite FTS5, conversation history)
        {
            let recall = self.recall.lock().await;
            match recall.search(query, limit) {
                Ok(entries) => {
                    for e in entries {
                        let preview = if e.content.len() > 200 {
                            let end = e
                                .content
                                .char_indices()
                                .nth(200)
                                .map(|(i, _)| i)
                                .unwrap_or(e.content.len());
                            format!("{}...", &e.content[..end])
                        } else {
                            e.content.clone()
                        };
                        all_results.push(format!(
                            "[recall:{}] {}: {}",
                            e.timestamp.format("%Y-%m-%d %H:%M"),
                            e.entry_type,
                            preview
                        ));
                    }
                }
                Err(e) => {
                    all_results.push(format!("[recall:search error] {e}"));
                }
            }
        }

        let elapsed = self.clock.mono_now().0.saturating_sub(start);

        if all_results.is_empty() {
            ToolResult {
                content: "No results found.".to_string(),
                is_error: false,
                metadata: ToolResultMeta {
                    execution_time_ms: elapsed,
                    truncated: false,
                    patch_delta: None,
                },
            }
        } else {
            let truncated = all_results.len() >= limit * 3;
            ToolResult {
                content: all_results.join("\n"),
                is_error: false,
                metadata: ToolResultMeta {
                    execution_time_ms: elapsed,
                    truncated,
                    patch_delta: None,
                },
            }
        }
    }
}
