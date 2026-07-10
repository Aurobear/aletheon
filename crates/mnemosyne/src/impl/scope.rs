//! MemoryScope -- multi-agent memory isolation.
//!
//! Provides visibility and write-control boundaries for memory blocks
//! in a parent-child agent hierarchy:
//!
//! - **Global**: shared across all agents; only the parent (owner) writes.
//! - **Session**: visible to parent and all children; children write only with approval.
//! - **Agent(id)**: private to a single agent; only that agent reads/writes.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::core_memory::{CoreMemory, MemoryBlock};

// ---------------------------------------------------------------------------
// MemoryScope
// ---------------------------------------------------------------------------

/// Visibility scope for a memory block in a multi-agent hierarchy.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MemoryScope {
    /// Shared across all agents. Only the parent (scope owner) may write.
    Global,
    /// Visible to parent and all child agents.
    /// Children write only with explicit approval (pending_write flag).
    Session,
    /// Private to a single agent identified by `String` (the agent id).
    Agent(String),
}

impl MemoryScope {
    /// Returns `true` if the given `agent_id` may read blocks in this scope.
    ///
    /// `is_parent` should be `true` when `agent_id` owns the scope (i.e. it is
    /// the top-level / orchestrator agent that created the memory).
    pub fn can_read(&self, agent_id: &str, _is_parent: bool) -> bool {
        match self {
            MemoryScope::Global => true,
            MemoryScope::Session => true,
            MemoryScope::Agent(id) => id == agent_id,
        }
    }

    /// Returns `true` if the given `agent_id` may directly write to blocks in
    /// this scope without requiring approval.
    pub fn can_write(&self, agent_id: &str, is_parent: bool) -> bool {
        match self {
            MemoryScope::Global => is_parent,
            MemoryScope::Session => is_parent,
            MemoryScope::Agent(id) => id == agent_id,
        }
    }

    /// Returns `true` if the given `agent_id` may *request* a write that
    /// requires parent approval (Session scope, child agent).
    pub fn can_request_write(&self, _agent_id: &str, is_parent: bool) -> bool {
        match self {
            MemoryScope::Session => !is_parent,
            _ => false,
        }
    }
}

// ---------------------------------------------------------------------------
// ScopedMemoryBlock
// ---------------------------------------------------------------------------

/// A memory block annotated with scope metadata for multi-agent isolation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScopedMemoryBlock {
    pub scope: MemoryScope,
    pub label: String,
    pub content: String,
    pub read_only: bool,
}

// ---------------------------------------------------------------------------
// PendingWrite -- child write requests for Session-scoped blocks
// ---------------------------------------------------------------------------

/// A pending write request from a child agent on a Session-scoped block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingWrite {
    pub agent_id: String,
    pub block_label: String,
    pub content: String,
    pub write_type: PendingWriteType,
}

/// The kind of write operation being requested.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PendingWriteType {
    /// Append `content` to the block.
    Append,
    /// Replace `old` with `content` inside the block.
    Replace { old: String },
    /// Replace entire block content.
    Rethink,
}

// ---------------------------------------------------------------------------
// ScopedCoreMemory
// ---------------------------------------------------------------------------

/// A `CoreMemory` wrapper that enforces `MemoryScope` visibility and write rules.
///
/// All read/write operations are gated by the caller's `agent_id` and whether
/// it is the parent (scope owner).
pub struct ScopedCoreMemory {
    inner: CoreMemory,
    scopes: HashMap<String, MemoryScope>,
    pending_writes: Vec<PendingWrite>,
}

impl ScopedCoreMemory {
    /// Wrap an existing `CoreMemory` with scope tracking.
    pub fn new(inner: CoreMemory) -> Self {
        Self {
            inner,
            scopes: HashMap::new(),
            pending_writes: Vec::new(),
        }
    }

    /// Register a scope for a block label. Call this when adding scoped blocks.
    pub fn set_scope(&mut self, label: &str, scope: MemoryScope) {
        self.scopes.insert(label.to_string(), scope);
    }

    /// Get the scope for a block, if registered. Unregistered blocks default
    /// to `Agent("unknown")` (effectively inaccessible).
    pub fn scope_of(&self, label: &str) -> MemoryScope {
        self.scopes
            .get(label)
            .cloned()
            .unwrap_or(MemoryScope::Agent("unknown".to_string()))
    }

    /// Add a scoped memory block. The block is inserted into the underlying
    /// `CoreMemory` and its scope is registered.
    pub fn add_block(&mut self, block: MemoryBlock, scope: MemoryScope) -> anyhow::Result<()> {
        let label = block.label.clone();
        self.inner.add_block(block)?;
        self.scopes.insert(label, scope);
        Ok(())
    }

    /// Read a block value if the agent has read permission.
    pub fn get(&self, label: &str, agent_id: &str, is_parent: bool) -> anyhow::Result<&str> {
        let scope = self.scope_of(label);
        if !scope.can_read(agent_id, is_parent) {
            anyhow::bail!(
                "Agent '{}' does not have read access to block '{}' (scope: {:?})",
                agent_id,
                label,
                scope
            );
        }
        self.inner
            .get(label)
            .ok_or_else(|| anyhow::anyhow!("Block '{}' not found", label))
    }

    /// List all block labels visible to the given agent.
    pub fn visible_blocks(&self, agent_id: &str, is_parent: bool) -> Vec<&str> {
        self.inner
            .blocks()
            .keys()
            .filter(|label| {
                let scope = self.scope_of(label);
                scope.can_read(agent_id, is_parent)
            })
            .map(|s| s.as_str())
            .collect()
    }

    /// Append to a block if the agent has write permission. Returns `Ok(true)`
    /// on success, or queues a `PendingWrite` if the agent can only request
    /// writes (Session scope, child).
    pub fn append(
        &mut self,
        label: &str,
        content: &str,
        agent_id: &str,
        is_parent: bool,
    ) -> anyhow::Result<WriteOutcome> {
        let scope = self.scope_of(label);
        if scope.can_write(agent_id, is_parent) {
            self.inner.append(label, content)?;
            Ok(WriteOutcome::Applied)
        } else if scope.can_request_write(agent_id, is_parent) {
            self.pending_writes.push(PendingWrite {
                agent_id: agent_id.to_string(),
                block_label: label.to_string(),
                content: content.to_string(),
                write_type: PendingWriteType::Append,
            });
            Ok(WriteOutcome::PendingApproval)
        } else {
            anyhow::bail!(
                "Agent '{}' does not have write access to block '{}' (scope: {:?})",
                agent_id,
                label,
                scope
            );
        }
    }

    /// Replace in a block if the agent has write permission.
    pub fn replace(
        &mut self,
        label: &str,
        old: &str,
        new: &str,
        agent_id: &str,
        is_parent: bool,
    ) -> anyhow::Result<WriteOutcome> {
        let scope = self.scope_of(label);
        if scope.can_write(agent_id, is_parent) {
            self.inner.replace(label, old, new)?;
            Ok(WriteOutcome::Applied)
        } else if scope.can_request_write(agent_id, is_parent) {
            self.pending_writes.push(PendingWrite {
                agent_id: agent_id.to_string(),
                block_label: label.to_string(),
                content: new.to_string(),
                write_type: PendingWriteType::Replace {
                    old: old.to_string(),
                },
            });
            Ok(WriteOutcome::PendingApproval)
        } else {
            anyhow::bail!(
                "Agent '{}' does not have write access to block '{}' (scope: {:?})",
                agent_id,
                label,
                scope
            );
        }
    }

    /// Rethink (replace entire content of) a block if the agent has write permission.
    pub fn rethink(
        &mut self,
        label: &str,
        new_content: &str,
        agent_id: &str,
        is_parent: bool,
    ) -> anyhow::Result<WriteOutcome> {
        let scope = self.scope_of(label);
        if scope.can_write(agent_id, is_parent) {
            self.inner.rethink(label, new_content)?;
            Ok(WriteOutcome::Applied)
        } else if scope.can_request_write(agent_id, is_parent) {
            self.pending_writes.push(PendingWrite {
                agent_id: agent_id.to_string(),
                block_label: label.to_string(),
                content: new_content.to_string(),
                write_type: PendingWriteType::Rethink,
            });
            Ok(WriteOutcome::PendingApproval)
        } else {
            anyhow::bail!(
                "Agent '{}' does not have write access to block '{}' (scope: {:?})",
                agent_id,
                label,
                scope
            );
        }
    }

    /// Drain all pending writes. Returns the list and clears the queue.
    pub fn drain_pending_writes(&mut self) -> Vec<PendingWrite> {
        std::mem::take(&mut self.pending_writes)
    }

    /// Approve a pending write by index, applying it to the underlying memory.
    /// Returns `false` if the index is out of bounds.
    pub fn approve_write(&mut self, index: usize) -> anyhow::Result<bool> {
        if index >= self.pending_writes.len() {
            return Ok(false);
        }
        let pw = self.pending_writes.remove(index);
        match &pw.write_type {
            PendingWriteType::Append => {
                self.inner.append(&pw.block_label, &pw.content)?;
            }
            PendingWriteType::Replace { old } => {
                self.inner.replace(&pw.block_label, old, &pw.content)?;
            }
            PendingWriteType::Rethink => {
                self.inner.rethink(&pw.block_label, &pw.content)?;
            }
        }
        Ok(true)
    }

    /// Reject (discard) a pending write by index.
    pub fn reject_write(&mut self, index: usize) -> bool {
        if index >= self.pending_writes.len() {
            return false;
        }
        self.pending_writes.remove(index);
        true
    }

    /// Access the underlying `CoreMemory` (bypasses scope checks -- for
    /// internal / parent use only).
    pub fn inner(&self) -> &CoreMemory {
        &self.inner
    }

    /// Mutable access to the underlying `CoreMemory`.
    pub fn inner_mut(&mut self) -> &mut CoreMemory {
        &mut self.inner
    }
}

/// Result of a scoped write operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteOutcome {
    /// Write was applied immediately.
    Applied,
    /// Write was queued for parent approval (Session scope, child agent).
    PendingApproval,
}

// ---------------------------------------------------------------------------
// ScopedRecallMemory -- scope-filtered recall queries
// ---------------------------------------------------------------------------

/// Scope filter for querying recall memory.
#[derive(Debug, Clone)]
pub enum ScopeFilter {
    /// Return entries from all scopes.
    All,
    /// Only entries belonging to a specific agent.
    Agent(String),
    /// Only session-level entries (shared between parent and children).
    Session,
    /// Only global entries.
    Global,
}

/// Extension trait for `RecallMemory` to support scope-filtered queries.
///
/// Since `RecallMemory` stores metadata as a JSON string, scope information
/// can be encoded in the metadata field. This helper parses and filters.
pub trait RecallScopeFilter {
    /// Filter a set of memory entries by scope metadata.
    fn filter_by_scope(
        entries: Vec<super::recall_memory::MemoryEntry>,
        filter: &ScopeFilter,
    ) -> Vec<super::recall_memory::MemoryEntry>;
}

/// Default scope filter implementation. Scope is stored in the metadata JSON
/// field as `{"scope": "global"}`, `{"scope": "session"}`, or
/// `{"scope": "agent:<id>"}`.
pub struct ScopedRecallFilter;

impl RecallScopeFilter for ScopedRecallFilter {
    fn filter_by_scope(
        entries: Vec<super::recall_memory::MemoryEntry>,
        filter: &ScopeFilter,
    ) -> Vec<super::recall_memory::MemoryEntry> {
        match filter {
            ScopeFilter::All => entries,
            ScopeFilter::Global => entries
                .into_iter()
                .filter(|e| matches_scope_metadata(&e.metadata, "global"))
                .collect(),
            ScopeFilter::Session => entries
                .into_iter()
                .filter(|e| matches_scope_metadata(&e.metadata, "session"))
                .collect(),
            ScopeFilter::Agent(id) => entries
                .into_iter()
                .filter(|e| matches_scope_metadata(&e.metadata, &format!("agent:{}", id)))
                .collect(),
        }
    }
}

/// Check if the metadata JSON string contains a matching `scope` field.
fn matches_scope_metadata(metadata: &Option<String>, expected: &str) -> bool {
    match metadata {
        Some(json_str) => {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(json_str) {
                val.get("scope")
                    .and_then(|s| s.as_str())
                    .map(|s| s == expected)
                    .unwrap_or(false)
            } else {
                false
            }
        }
        None => false,
    }
}

/// Helper to build scope metadata JSON string for recall entries.
pub fn scope_metadata(scope: &MemoryScope) -> String {
    let scope_str = match scope {
        MemoryScope::Global => "global".to_string(),
        MemoryScope::Session => "session".to_string(),
        MemoryScope::Agent(id) => format!("agent:{}", id),
    };
    serde_json::json!({ "scope": scope_str }).to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- MemoryScope permission tests --

    #[test]
    fn test_global_scope_permissions() {
        let scope = MemoryScope::Global;
        // Everyone can read
        assert!(scope.can_read("child-1", false));
        assert!(scope.can_read("parent", true));
        // Only parent writes
        assert!(scope.can_write("parent", true));
        assert!(!scope.can_write("child-1", false));
        // Children cannot request write on Global
        assert!(!scope.can_request_write("child-1", false));
    }

    #[test]
    fn test_session_scope_permissions() {
        let scope = MemoryScope::Session;
        // Everyone can read
        assert!(scope.can_read("parent", true));
        assert!(scope.can_read("child-1", false));
        // Only parent writes directly
        assert!(scope.can_write("parent", true));
        assert!(!scope.can_write("child-1", false));
        // Children can request write
        assert!(scope.can_request_write("child-1", false));
        assert!(!scope.can_request_write("parent", true));
    }

    #[test]
    fn test_agent_scope_permissions() {
        let scope = MemoryScope::Agent("agent-42".to_string());
        // Only owner reads
        assert!(scope.can_read("agent-42", false));
        assert!(!scope.can_read("other-agent", false));
        assert!(!scope.can_read("parent", true));
        // Only owner writes
        assert!(scope.can_write("agent-42", false));
        assert!(!scope.can_write("other-agent", false));
        assert!(!scope.can_write("parent", true));
        // No request write on Agent scope
        assert!(!scope.can_request_write("agent-42", false));
        assert!(!scope.can_request_write("parent", true));
    }

    // -- ScopedCoreMemory tests --

    #[test]
    fn test_scoped_core_memory_read_access() {
        let core = CoreMemory::with_defaults();
        let mut scoped = ScopedCoreMemory::new(core);
        scoped.set_scope("persona", MemoryScope::Global);
        scoped.set_scope("system_state", MemoryScope::Session);
        scoped.set_scope("user_prefs", MemoryScope::Agent("agent-1".to_string()));

        // Global: anyone reads
        assert!(scoped.get("persona", "child", false).is_ok());
        assert!(scoped.get("persona", "parent", true).is_ok());

        // Session: anyone reads
        assert!(scoped.get("system_state", "parent", true).is_ok());
        assert!(scoped.get("system_state", "child", false).is_ok());

        // Agent: only owner reads
        assert!(scoped.get("user_prefs", "agent-1", false).is_ok());
        assert!(scoped.get("user_prefs", "parent", true).is_err());
        assert!(scoped.get("user_prefs", "other", false).is_err());
    }

    #[test]
    fn test_scoped_core_memory_write_global_parent_only() {
        let core = CoreMemory::new();
        let mut scoped = ScopedCoreMemory::new(core);
        scoped
            .add_block(
                MemoryBlock::new("shared", "initial", 1000),
                MemoryScope::Global,
            )
            .unwrap();

        // Parent can write
        let outcome = scoped.append("shared", "data", "parent", true).unwrap();
        assert_eq!(outcome, WriteOutcome::Applied);

        // Child cannot write
        assert!(scoped.append("shared", "data", "child", false).is_err());
    }

    #[test]
    fn test_scoped_core_memory_session_child_write_pending() {
        let core = CoreMemory::new();
        let mut scoped = ScopedCoreMemory::new(core);
        scoped
            .add_block(
                MemoryBlock::new("session_info", "", 1000),
                MemoryScope::Session,
            )
            .unwrap();

        // Parent writes directly
        let outcome = scoped
            .append("session_info", "from-parent", "parent", true)
            .unwrap();
        assert_eq!(outcome, WriteOutcome::Applied);

        // Child write is queued
        let outcome = scoped
            .append("session_info", "from-child", "child-1", false)
            .unwrap();
        assert_eq!(outcome, WriteOutcome::PendingApproval);

        // Pending writes are tracked
        let pending = scoped.drain_pending_writes();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].agent_id, "child-1");
        assert_eq!(pending[0].block_label, "session_info");
    }

    #[test]
    fn test_scoped_core_memory_approve_and_reject_writes() {
        let core = CoreMemory::new();
        let mut scoped = ScopedCoreMemory::new(core);
        scoped
            .add_block(
                MemoryBlock::new("task_info", "", 1000),
                MemoryScope::Session,
            )
            .unwrap();

        // Queue two child writes
        scoped
            .append("task_info", "step-1", "child-a", false)
            .unwrap();
        scoped
            .append("task_info", "step-2", "child-b", false)
            .unwrap();

        // Drain returns both and clears the queue
        let pending = scoped.drain_pending_writes();
        assert_eq!(pending.len(), 2);
        assert_eq!(scoped.drain_pending_writes().len(), 0);

        // Re-queue for approve/reject test
        scoped
            .append("task_info", "step-1", "child-a", false)
            .unwrap();
        scoped
            .append("task_info", "step-2", "child-b", false)
            .unwrap();

        // Reject first, approve second
        assert!(scoped.reject_write(0));
        assert!(scoped.approve_write(0).unwrap());

        // Only the approved content should be in the block
        assert_eq!(scoped.get("task_info", "parent", true).unwrap(), "step-2");
    }

    #[test]
    fn test_scoped_core_memory_visible_blocks() {
        let core = CoreMemory::new();
        let mut scoped = ScopedCoreMemory::new(core);
        scoped
            .add_block(
                MemoryBlock::new("global_info", "g", 100),
                MemoryScope::Global,
            )
            .unwrap();
        scoped
            .add_block(
                MemoryBlock::new("session_info", "s", 100),
                MemoryScope::Session,
            )
            .unwrap();
        scoped
            .add_block(
                MemoryBlock::new("agent_1_info", "a1", 100),
                MemoryScope::Agent("agent-1".to_string()),
            )
            .unwrap();
        scoped
            .add_block(
                MemoryBlock::new("agent_2_info", "a2", 100),
                MemoryScope::Agent("agent-2".to_string()),
            )
            .unwrap();

        // Parent sees global + session only (Agent-scoped blocks are private)
        let parent_blocks = scoped.visible_blocks("parent", true);
        assert_eq!(parent_blocks.len(), 2);
        assert!(parent_blocks.contains(&"global_info"));
        assert!(parent_blocks.contains(&"session_info"));

        // Child sees global + session + own private
        let child1_blocks = scoped.visible_blocks("agent-1", false);
        assert_eq!(child1_blocks.len(), 3);
        assert!(child1_blocks.contains(&"global_info"));
        assert!(child1_blocks.contains(&"session_info"));
        assert!(child1_blocks.contains(&"agent_1_info"));
        assert!(!child1_blocks.contains(&"agent_2_info"));
    }

    #[test]
    fn test_agent_scope_isolation_between_children() {
        let core = CoreMemory::new();
        let mut scoped = ScopedCoreMemory::new(core);
        scoped
            .add_block(
                MemoryBlock::new("notes", "", 500),
                MemoryScope::Agent("agent-A".to_string()),
            )
            .unwrap();

        // agent-A can read/write
        assert!(scoped.get("notes", "agent-A", false).is_ok());
        let outcome = scoped.append("notes", "my data", "agent-A", false).unwrap();
        assert_eq!(outcome, WriteOutcome::Applied);

        // agent-B cannot read or write
        assert!(scoped.get("notes", "agent-B", false).is_err());
        assert!(scoped
            .append("notes", "intrusion", "agent-B", false)
            .is_err());
    }

    // -- ScopeFilter / recall scope tests --

    #[test]
    fn test_scope_metadata_helper() {
        assert_eq!(
            scope_metadata(&MemoryScope::Global),
            r#"{"scope":"global"}"#
        );
        assert_eq!(
            scope_metadata(&MemoryScope::Session),
            r#"{"scope":"session"}"#
        );
        assert_eq!(
            scope_metadata(&MemoryScope::Agent("x".into())),
            r#"{"scope":"agent:x"}"#
        );
    }

    #[test]
    fn test_recall_scope_filter() {
        use super::super::recall_memory::MemoryEntry;
        use chrono::Utc;

        let make_entry = |scope_json: &str| MemoryEntry {
            id: 1,
            timestamp: Utc::now(),
            session_id: "s1".into(),
            entry_type: "msg".into(),
            content: "test".into(),
            metadata: Some(scope_json.to_string()),
        };

        let entries = vec![
            make_entry(r#"{"scope":"global"}"#),
            make_entry(r#"{"scope":"session"}"#),
            make_entry(r#"{"scope":"agent:agent-1"}"#),
            make_entry(r#"{"scope":"agent:agent-2"}"#),
            make_entry(r#"{"other":"field"}"#), // no scope
        ];

        let all = ScopedRecallFilter::filter_by_scope(entries.clone(), &ScopeFilter::All);
        assert_eq!(all.len(), 5);

        let global = ScopedRecallFilter::filter_by_scope(entries.clone(), &ScopeFilter::Global);
        assert_eq!(global.len(), 1);

        let session = ScopedRecallFilter::filter_by_scope(entries.clone(), &ScopeFilter::Session);
        assert_eq!(session.len(), 1);

        let a1 = ScopedRecallFilter::filter_by_scope(
            entries.clone(),
            &ScopeFilter::Agent("agent-1".into()),
        );
        assert_eq!(a1.len(), 1);

        let a2 =
            ScopedRecallFilter::filter_by_scope(entries, &ScopeFilter::Agent("agent-2".into()));
        assert_eq!(a2.len(), 1);
    }

    // -- ScopedCoreMemory: replace and rethink with scope --

    #[test]
    fn test_scoped_replace_and_rethink() {
        let core = CoreMemory::new();
        let mut scoped = ScopedCoreMemory::new(core);
        scoped
            .add_block(
                MemoryBlock::new("notes", "hello world", 500),
                MemoryScope::Agent("owner".to_string()),
            )
            .unwrap();

        // Replace works for owner
        let outcome = scoped
            .replace("notes", "hello", "goodbye", "owner", false)
            .unwrap();
        assert_eq!(outcome, WriteOutcome::Applied);
        assert_eq!(
            scoped.get("notes", "owner", false).unwrap(),
            "goodbye world"
        );

        // Rethink works for owner
        let outcome = scoped
            .rethink("notes", "completely new", "owner", false)
            .unwrap();
        assert_eq!(outcome, WriteOutcome::Applied);
        assert_eq!(
            scoped.get("notes", "owner", false).unwrap(),
            "completely new"
        );

        // Other agent cannot replace
        assert!(scoped
            .replace("notes", "new", "intrusion", "other", false)
            .is_err());
    }

    // -- Session scope: child replace queues as pending --

    #[test]
    fn test_session_child_replace_pending() {
        let core = CoreMemory::new();
        let mut scoped = ScopedCoreMemory::new(core);
        scoped
            .add_block(
                MemoryBlock::new("shared", "alpha beta", 500),
                MemoryScope::Session,
            )
            .unwrap();

        // Child replace is pending
        let outcome = scoped
            .replace("shared", "alpha", "gamma", "child-x", false)
            .unwrap();
        assert_eq!(outcome, WriteOutcome::PendingApproval);
        let pending = scoped.drain_pending_writes();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].agent_id, "child-x");
    }
}
