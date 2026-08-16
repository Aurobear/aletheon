//! Pre-summarization tool output pruning.
//!
//! Moved to `::contracts::compaction` (the shared context-compaction interface)
//! so both `corpus` and `mnemosyne` can use it without one depending on
//! the other. Re-exported here to keep the existing public path working.
pub use runtime::compaction::prune_tool_outputs;
