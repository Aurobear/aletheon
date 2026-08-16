//! Per-turn file delta accumulation and model-context projection.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TurnDiffTracker {
    files: HashMap<String, TurnFileDelta>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TurnFileDelta {
    pub edits: usize,
    pub hunks_applied: usize,
    pub bytes_before: u64,
    pub bytes_after: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnFileDeltaSnapshot {
    pub path: String,
    pub edits: usize,
    pub hunks_applied: usize,
    pub bytes_before: u64,
    pub bytes_after: u64,
}

impl TurnDiffTracker {
    pub fn record_patch_delta(&mut self, delta: &::contracts::PatchDelta) {
        for change in &delta.files_changed {
            let entry = self
                .files
                .entry(change.path.clone())
                .or_insert_with(|| TurnFileDelta {
                    bytes_before: change.bytes_before,
                    ..Default::default()
                });
            entry.edits = entry.edits.saturating_add(1);
            entry.hunks_applied = entry.hunks_applied.saturating_add(change.hunks_applied);
            entry.bytes_after = change.bytes_after;
        }
    }

    pub fn record_file_write(&mut self, path: &str, bytes_written: u64) {
        let entry = self.files.entry(path.to_owned()).or_default();
        entry.edits = entry.edits.saturating_add(1);
        entry.hunks_applied = entry.hunks_applied.saturating_add(1);
        entry.bytes_after = bytes_written;
    }

    pub fn to_context_injection(&self) -> String {
        if self.files.is_empty() {
            return String::new();
        }
        let mut files: Vec<_> = self.files.iter().collect();
        files.sort_by_key(|(path, _)| *path);
        let hunks = files
            .iter()
            .map(|(_, delta)| delta.hunks_applied)
            .sum::<usize>();
        let mut output = String::from(
            "## Files changed this turn\n\n| File | Edits | Hunks | Size |\n|------|-------|-------|------|\n",
        );
        for (path, delta) in &files {
            output.push_str(&format!(
                "| `{}` | {} | {} | {}B → {}B |\n",
                path.replace('`', "\\`"),
                delta.edits,
                delta.hunks_applied,
                delta.bytes_before,
                delta.bytes_after,
            ));
        }
        output.push_str(&format!(
            "\n{} files changed, {} hunks applied.",
            files.len(),
            hunks
        ));
        output
    }

    pub fn snapshot(&self) -> Vec<TurnFileDeltaSnapshot> {
        let mut snapshot = self
            .files
            .iter()
            .map(|(path, delta)| TurnFileDeltaSnapshot {
                path: path.clone(),
                edits: delta.edits,
                hunks_applied: delta.hunks_applied,
                bytes_before: delta.bytes_before,
                bytes_after: delta.bytes_after,
            })
            .collect::<Vec<_>>();
        snapshot.sort_by(|left, right| left.path.cmp(&right.path));
        snapshot
    }

    pub fn reset(&mut self) {
        self.files.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn patch(path: &str, before: u64, after: u64, hunks: usize) -> ::contracts::PatchDelta {
        ::contracts::PatchDelta {
            files_changed: vec![::contracts::PatchDeltaFileChange {
                path: path.into(),
                change_type: "modified".into(),
                hunks_applied: hunks,
                bytes_before: before,
                bytes_after: after,
                is_binary: false,
            }],
            ..Default::default()
        }
    }

    #[test]
    fn accumulates_multiple_patch_results_for_the_same_file() {
        let mut tracker = TurnDiffTracker::default();
        tracker.record_patch_delta(&patch("src/main.rs", 100, 120, 2));
        tracker.record_patch_delta(&patch("src/main.rs", 120, 150, 3));
        assert_eq!(
            tracker.files.get("src/main.rs"),
            Some(&TurnFileDelta {
                edits: 2,
                hunks_applied: 5,
                bytes_before: 100,
                bytes_after: 150,
            })
        );
    }

    #[test]
    fn context_injection_is_a_stable_markdown_table() {
        let mut tracker = TurnDiffTracker::default();
        tracker.record_patch_delta(&patch("src/main.rs", 100, 150, 2));
        tracker.record_file_write("Cargo.toml", 240);
        let context = tracker.to_context_injection();
        assert!(context.starts_with("## Files changed this turn"));
        assert!(context.contains("| `Cargo.toml` | 1 | 1 | 0B → 240B |"));
        assert!(context.contains("| `src/main.rs` | 1 | 2 | 100B → 150B |"));
        assert!(context.ends_with("2 files changed, 3 hunks applied."));
    }

    #[test]
    fn snapshot_is_sorted_and_does_not_expose_internal_map() {
        let mut tracker = TurnDiffTracker::default();
        tracker.record_file_write("z.rs", 2);
        tracker.record_file_write("a.rs", 1);
        let snapshot = tracker.snapshot();
        assert_eq!(snapshot[0].path, "a.rs");
        assert_eq!(snapshot[1].path, "z.rs");
    }
}

#[cfg(test)]
mod configuration_tests {
    #[test]
    fn code_agent_grants_structured_editing_tool_set() {
        let manifest = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../agents/code-agent.toml"
        ))
        .unwrap();
        for tool in ["code_graph", "grep", "glob", "apply_patch"] {
            assert!(
                manifest.contains(&format!("\"{tool}\"")),
                "code-agent is missing {tool}"
            );
        }
    }
}
