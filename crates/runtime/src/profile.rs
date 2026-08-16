//! Runtime-owned Agent profile document contract (RA-05).
//!
//! Markdown is only one composition adapter. The semantic profile document
//! lives in Runtime so a loader cannot become a second Agent authority.

use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AgentProfileDocument {
    pub name: String,
    pub description: String,
    pub tools: Vec<String>,
    pub delegate_tools: Option<Vec<String>>,
    pub model: Option<String>,
    pub max_iterations: usize,
    pub body: String,
    pub path: PathBuf,
}
