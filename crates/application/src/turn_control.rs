//! Application-owned turn interruption and collaboration policy vocabulary.

use serde::{Deserialize, Serialize};

pub use contracts::turn_control::InterruptReason;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CollaborationMode {
    #[default]
    Default,
    Plan,
    Auto,
    Sandbox,
}

impl CollaborationMode {
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Plan => "plan",
            Self::Auto => "auto",
            Self::Sandbox => "sandbox",
        }
    }

    pub const fn system_prompt_suffix(self) -> &'static str {
        match self {
            Self::Default => "Operate normally. Ask for user approval before destructive operations.",
            Self::Plan => "You are in PLAN MODE. You may only use read-only tools (glob, grep, read, web_fetch). Generate a detailed plan. Do NOT execute any mutations. Wait for user approval before proceeding.",
            Self::Auto => "You are in AUTO MODE. Execute without asking for approval. Be thorough and autonomous. Persist until the task is fully handled.",
            Self::Sandbox => "You are in SANDBOX MODE. All side-effect operations run in a sandbox first. Review sandbox results before applying to the real environment.",
        }
    }
}
