use std::collections::HashMap;
use serde::{Deserialize, Serialize};

/// A single memory block in Core Memory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryBlock {
    pub label: String,
    pub value: String,
    pub char_limit: usize,
    pub read_only: bool,
}

impl MemoryBlock {
    pub fn new(label: impl Into<String>, value: impl Into<String>, char_limit: usize) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
            char_limit,
            read_only: false,
        }
    }

    pub fn read_only(label: impl Into<String>, value: impl Into<String>, char_limit: usize) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
            char_limit,
            read_only: true,
        }
    }

    pub fn remaining_capacity(&self) -> usize {
        self.char_limit.saturating_sub(self.value.len())
    }
}

/// L1 Core Memory -- editable blocks in context window.
/// Inspired by Letta (MemGPT) core_memory.
pub struct CoreMemory {
    blocks: HashMap<String, MemoryBlock>,
}

impl CoreMemory {
    pub fn new() -> Self {
        Self {
            blocks: HashMap::new(),
        }
    }

    /// Initialize with default blocks.
    pub fn with_defaults() -> Self {
        let mut memory = Self::new();
        memory.blocks.insert("persona".to_string(), MemoryBlock::read_only(
            "persona",
            "You are a helpful system assistant running as a daemon on Arch Linux.",
            2000,
        ));
        memory.blocks.insert("system_state".to_string(), MemoryBlock::new(
            "system_state",
            "",
            2000,
        ));
        memory.blocks.insert("user_prefs".to_string(), MemoryBlock::new(
            "user_prefs",
            "",
            1000,
        ));
        memory
    }

    /// Add a new block.
    pub fn add_block(&mut self, block: MemoryBlock) -> anyhow::Result<()> {
        if self.blocks.contains_key(&block.label) {
            anyhow::bail!("Block '{}' already exists", block.label);
        }
        self.blocks.insert(block.label.clone(), block);
        Ok(())
    }

    /// Get a block value.
    pub fn get(&self, label: &str) -> Option<&str> {
        self.blocks.get(label).map(|b| b.value.as_str())
    }

    /// Get all blocks.
    pub fn blocks(&self) -> &HashMap<String, MemoryBlock> {
        &self.blocks
    }

    /// Append content to a block (Letta: core_memory_append).
    pub fn append(&mut self, label: &str, content: &str) -> anyhow::Result<()> {
        let block = self.blocks.get_mut(label)
            .ok_or_else(|| anyhow::anyhow!("Block '{}' not found", label))?;

        if block.read_only {
            anyhow::bail!("Block '{}' is read-only", label);
        }

        let new_value = if block.value.is_empty() {
            content.to_string()
        } else {
            format!("{}\n{}", block.value, content)
        };

        if new_value.len() > block.char_limit {
            anyhow::bail!(
                "Block '{}' would exceed limit ({}/{} chars)",
                label, new_value.len(), block.char_limit
            );
        }

        block.value = new_value;
        Ok(())
    }

    /// Replace content in a block (Letta: core_memory_replace).
    pub fn replace(&mut self, label: &str, old: &str, new: &str) -> anyhow::Result<()> {
        let block = self.blocks.get_mut(label)
            .ok_or_else(|| anyhow::anyhow!("Block '{}' not found", label))?;

        if block.read_only {
            anyhow::bail!("Block '{}' is read-only", label);
        }

        if !block.value.contains(old) {
            anyhow::bail!("Block '{}' does not contain '{}'", label, old);
        }

        block.value = block.value.replacen(old, new, 1);
        Ok(())
    }

    /// Replace entire block content (Letta: core_memory_rethink).
    pub fn rethink(&mut self, label: &str, new_content: &str) -> anyhow::Result<()> {
        let block = self.blocks.get_mut(label)
            .ok_or_else(|| anyhow::anyhow!("Block '{}' not found", label))?;

        if block.read_only {
            anyhow::bail!("Block '{}' is read-only", label);
        }

        if new_content.len() > block.char_limit {
            anyhow::bail!(
                "New content exceeds limit ({}/{} chars)",
                new_content.len(), block.char_limit
            );
        }

        block.value = new_content.to_string();
        Ok(())
    }

    /// Serialize all blocks to JSON for persistence.
    pub fn to_json(&self) -> anyhow::Result<String> {
        Ok(serde_json::to_string_pretty(&self.blocks)?)
    }

    /// Load blocks from JSON.
    pub fn from_json(json: &str) -> anyhow::Result<Self> {
        let blocks: HashMap<String, MemoryBlock> = serde_json::from_str(json)?;
        Ok(Self { blocks })
    }

    /// Format all blocks for injection into LLM context.
    pub fn format_for_context(&self) -> String {
        let mut parts = Vec::new();
        for (label, block) in &self.blocks {
            parts.push(format!("[{}]:\n{}", label, block.value));
        }
        parts.join("\n\n")
    }
}

impl Default for CoreMemory {
    fn default() -> Self {
        Self::new()
    }
}
