use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum RuntimeEvent {
    Started { session_id: String },
    ToolRequested(ToolRequestEvent),
    ToolStarted { name: String },
    ToolCompleted { name: String, success: bool },
    CommandOutput(CommandOutputEvent),
    FileChanged { path: String },
    Settled { success: bool },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolRequestEvent { pub name: String, pub input: serde_json::Value }

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CommandOutputEvent { pub command: String, pub stdout: String, pub stderr: String, pub exit_code: i32 }
