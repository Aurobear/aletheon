use aletheon_abi::ReflectionEntry;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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

    pub fn read_only(
        label: impl Into<String>,
        value: impl Into<String>,
        char_limit: usize,
    ) -> Self {
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
        memory.blocks.insert(
            "persona".to_string(),
            MemoryBlock::read_only(
                "persona",
                "You are a helpful system assistant running as a daemon on Arch Linux.",
                2000,
            ),
        );
        memory.blocks.insert(
            "system_state".to_string(),
            MemoryBlock::new("system_state", "", 2000),
        );
        memory.blocks.insert(
            "user_prefs".to_string(),
            MemoryBlock::new("user_prefs", "", 1000),
        );
        memory
            .blocks
            .insert("human".to_string(), MemoryBlock::new("human", "", 2000));
        memory
            .blocks
            .insert("learned".to_string(), MemoryBlock::new("learned", "", 3000));
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

    /// Insert or replace a block unconditionally.
    pub fn set_block(&mut self, block: MemoryBlock) {
        self.blocks.insert(block.label.clone(), block);
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
        let block = self
            .blocks
            .get_mut(label)
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
                label,
                new_value.len(),
                block.char_limit
            );
        }

        block.value = new_value;
        Ok(())
    }

    /// Replace content in a block (Letta: core_memory_replace).
    pub fn replace(&mut self, label: &str, old: &str, new: &str) -> anyhow::Result<()> {
        let block = self
            .blocks
            .get_mut(label)
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
        let block = self
            .blocks
            .get_mut(label)
            .ok_or_else(|| anyhow::anyhow!("Block '{}' not found", label))?;

        if block.read_only {
            anyhow::bail!("Block '{}' is read-only", label);
        }

        if new_content.len() > block.char_limit {
            anyhow::bail!(
                "New content exceeds limit ({}/{} chars)",
                new_content.len(),
                block.char_limit
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

    /// Auto-populate "human" and "learned" blocks from recent reflections.
    ///
    /// Extracts user patterns/preferences into "human" and agent knowledge
    /// into "learned". Respects char_limit by truncating oldest entries when
    /// the block would overflow.
    pub fn auto_populate_learned(&mut self, reflections: &[ReflectionEntry]) {
        for entry in reflections {
            // Extract user patterns from what_worked into "human" block
            for lesson in &entry.what_worked {
                let snippet = format!("- {}", lesson);
                if let Some(block) = self.blocks.get("human") {
                    if block.remaining_capacity() < snippet.len() + 1 {
                        self.truncate_oldest("human", snippet.len() + 1);
                    }
                }
                // Ignore errors (block not found, etc.)
                let _ = self.append("human", &snippet);
            }

            // Extract agent knowledge from learned into "learned" block
            for lesson in &entry.learned {
                let snippet = format!("- {}", lesson);
                if let Some(block) = self.blocks.get("learned") {
                    if block.remaining_capacity() < snippet.len() + 1 {
                        self.truncate_oldest("learned", snippet.len() + 1);
                    }
                }
                let _ = self.append("learned", &snippet);
            }
        }
    }

    /// Truncate oldest entries from a block to make room for `needed` chars.
    /// Removes lines from the front of the block value.
    fn truncate_oldest(&mut self, label: &str, needed: usize) {
        let (new_value, limit) = {
            let block = match self.blocks.get(label) {
                Some(b) => b,
                None => return,
            };

            let limit = block.char_limit;
            let mut lines: Vec<&str> = block.value.lines().collect();

            // Remove oldest entries (front of list) until we have room
            while !lines.is_empty() {
                let current_len: usize = lines.iter().map(|l| l.len() + 1).sum::<usize>(); // +1 for newline
                if current_len + needed <= limit {
                    break;
                }
                lines.remove(0);
            }

            (lines.join("\n"), limit)
        };

        // Compute the new value outside the borrow, then apply
        let trimmed = {
            // Ensure we stay within limit (the new_value already accounts for truncation)
            if new_value.len() + needed > limit {
                // Emergency: clear the block entirely
                String::new()
            } else {
                new_value
            }
        };

        if let Some(block) = self.blocks.get_mut(label) {
            block.value = trimmed;
        }
    }

    /// Format all blocks for system prompt injection with friendly section names.
    ///
    /// Output ordering: persona, human, learned, system_state, user_prefs.
    /// Blocks not present are silently skipped.
    pub fn inject_into_prompt(&self) -> String {
        let section_order = [
            ("persona", "Persona"),
            ("human", "User Profile"),
            ("learned", "Learned Knowledge"),
            ("system_state", "System State"),
            ("user_prefs", "User Preferences"),
        ];

        let mut parts = Vec::new();
        for (key, title) in &section_order {
            if let Some(block) = self.blocks.get(*key) {
                parts.push(format!("[{}]\n{}", title, block.value));
            }
        }

        // Append any extra blocks not in the canonical order
        for (label, block) in &self.blocks {
            if !section_order.iter().any(|(k, _)| k == label) {
                parts.push(format!("[{}]\n{}", label, block.value));
            }
        }

        parts.join("\n\n")
    }
}

impl Default for CoreMemory {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aletheon_abi::{ReflectionOutcome, ReflectionTrigger};

    fn make_reflection(what_worked: Vec<&str>, learned: Vec<&str>) -> ReflectionEntry {
        ReflectionEntry {
            id: "test-1".to_string(),
            timestamp: chrono::Utc::now(),
            trigger: ReflectionTrigger::TaskComplete,
            task_summary: "test task".to_string(),
            outcome: ReflectionOutcome::Success,
            what_worked: what_worked.into_iter().map(String::from).collect(),
            what_failed: vec![],
            learned: learned.into_iter().map(String::from).collect(),
            behavior_changes: vec![],
            confidence: 0.9,
        }
    }

    #[test]
    fn with_defaults_has_five_blocks() {
        let mem = CoreMemory::with_defaults();
        assert_eq!(mem.blocks().len(), 5);
        assert!(mem.blocks().contains_key("persona"));
        assert!(mem.blocks().contains_key("system_state"));
        assert!(mem.blocks().contains_key("user_prefs"));
        assert!(mem.blocks().contains_key("human"));
        assert!(mem.blocks().contains_key("learned"));
    }

    #[test]
    fn persona_is_read_only() {
        let mem = CoreMemory::with_defaults();
        assert!(mem.blocks().get("persona").unwrap().read_only);
    }

    #[test]
    fn human_and_learned_are_writable() {
        let mem = CoreMemory::with_defaults();
        assert!(!mem.blocks().get("human").unwrap().read_only);
        assert!(!mem.blocks().get("learned").unwrap().read_only);
    }

    #[test]
    fn auto_populate_learned_populates_blocks() {
        let mut mem = CoreMemory::with_defaults();
        let reflections = vec![make_reflection(
            vec!["use short prompts", "check disk first"],
            vec!["rust borrow checker prefers explicit lifetimes"],
        )];

        mem.auto_populate_learned(&reflections);

        let human = mem.get("human").unwrap();
        assert!(human.contains("use short prompts"));
        assert!(human.contains("check disk first"));

        let learned = mem.get("learned").unwrap();
        assert!(learned.contains("rust borrow checker prefers explicit lifetimes"));
    }

    #[test]
    fn auto_populate_respects_char_limit() {
        let mut mem = CoreMemory::with_defaults();
        // Set a very small limit on "learned" to test truncation
        mem.blocks.get_mut("learned").unwrap().char_limit = 60;

        let reflections = vec![
            make_reflection(vec![], vec!["first lesson"]),
            make_reflection(vec![], vec!["second lesson"]),
            make_reflection(
                vec![],
                vec!["third very long lesson that should push out the oldest"],
            ),
        ];

        mem.auto_populate_learned(&reflections);

        let learned = mem.get("learned").unwrap();
        // The oldest entry should have been truncated
        assert!(!learned.contains("first lesson"));
        // The newest entries should remain
        assert!(learned.len() <= 60);
    }

    #[test]
    fn inject_into_prompt_has_correct_format() {
        let mut mem = CoreMemory::with_defaults();
        mem.append("system_state", "uptime: 3h").unwrap();

        let prompt = mem.inject_into_prompt();
        assert!(prompt.contains("[Persona]"));
        assert!(prompt.contains("[User Profile]"));
        assert!(prompt.contains("[Learned Knowledge]"));
        assert!(prompt.contains("[System State]"));
        assert!(prompt.contains("[User Preferences]"));
        assert!(prompt.contains("uptime: 3h"));

        // Check ordering: Persona before User Profile before Learned Knowledge
        let pos_persona = prompt.find("[Persona]").unwrap();
        let pos_human = prompt.find("[User Profile]").unwrap();
        let pos_learned = prompt.find("[Learned Knowledge]").unwrap();
        let pos_sys = prompt.find("[System State]").unwrap();
        let pos_prefs = prompt.find("[User Preferences]").unwrap();
        assert!(pos_persona < pos_human);
        assert!(pos_human < pos_learned);
        assert!(pos_learned < pos_sys);
        assert!(pos_sys < pos_prefs);
    }

    #[test]
    fn inject_into_prompt_skips_missing_blocks() {
        let mem = CoreMemory::new();
        // No blocks at all — should return empty string
        let prompt = mem.inject_into_prompt();
        assert!(prompt.is_empty());
    }

    #[test]
    fn inject_includes_extra_blocks() {
        let mut mem = CoreMemory::with_defaults();
        mem.add_block(MemoryBlock::new("custom", "custom data", 500))
            .unwrap();

        let prompt = mem.inject_into_prompt();
        assert!(prompt.contains("[custom]"));
        assert!(prompt.contains("custom data"));
    }

    #[test]
    fn empty_reflections_do_not_crash() {
        let mut mem = CoreMemory::with_defaults();
        mem.auto_populate_learned(&[]);
        // Blocks should remain empty
        assert_eq!(mem.get("human"), Some(""));
        assert_eq!(mem.get("learned"), Some(""));
    }
}
