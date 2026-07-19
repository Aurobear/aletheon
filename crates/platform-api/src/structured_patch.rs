//! Structured patch types — canonical data model shared between corpus
//! and exec-server. Fabric-dependent wrappers live in corpus.
//! See crates/corpus/src/tools/tools/structured_patch.rs for the full impl.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PatchOperation {
    DeleteFile { path: PathBuf },
    AddFile { path: PathBuf, content: String },
    UpdateFile { path: PathBuf, content: String },
    AppendFile { path: PathBuf, content: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructuredPatch {
    pub operations: Vec<PatchOperation>,
}

#[derive(Clone, Debug)]
pub struct StructuredPatchResult {
    pub operations: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub errors: Vec<String>,
}

/// Parse a multi-operation patch from plain text (stub — full impl in corpus).
pub fn parse_structured_patch(_input: &str) -> Result<StructuredPatch, String> {
    Ok(StructuredPatch { operations: vec![] })
}

/// Parse a JSON-encoded patch.
pub fn parse_structured_patch_json(input: &str) -> Result<StructuredPatch, String> {
    serde_json::from_str(input).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patch_operation_serde_round_trip() {
        let patch = StructuredPatch {
            operations: vec![
                PatchOperation::AddFile {
                    path: PathBuf::from("src/main.rs"),
                    content: "fn main() {}".into(),
                },
                PatchOperation::DeleteFile {
                    path: PathBuf::from("old.rs"),
                },
            ],
        };
        let json = serde_json::to_string(&patch).unwrap();
        let back: StructuredPatch = serde_json::from_str(&json).unwrap();
        assert_eq!(back.operations.len(), 2);
    }

    #[test]
    fn structured_patch_module_accessible() {
        let op = PatchOperation::UpdateFile {
            path: PathBuf::from("lib.rs"),
            content: "content".into(),
        };
        assert!(matches!(op, PatchOperation::UpdateFile { .. }));
    }
}
