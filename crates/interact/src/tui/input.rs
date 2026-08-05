use std::{fs, path::PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use fabric::WorkspacePolicy;

const HISTORY_SCHEMA_VERSION: u16 = 1;
const MAX_HISTORY_ENTRIES: usize = 50;
const MAX_HISTORY_ENTRY_BYTES: usize = 128 * 1024;
const MAX_DRAFT_BYTES: usize = 1_000_000;

pub struct CommandHistory {
    entries: Vec<String>,
    cursor: usize,
    max_size: usize,
}

impl Default for CommandHistory {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandHistory {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            cursor: 0,
            max_size: 50,
        }
    }

    pub fn push(&mut self, entry: String) {
        if entry.is_empty() {
            return;
        }
        if self.entries.last() == Some(&entry) {
            return;
        }
        self.entries.push(entry);
        if self.entries.len() > self.max_size {
            self.entries.remove(0);
        }
        self.cursor = self.entries.len();
    }

    pub fn up(&mut self) -> Option<&str> {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.entries.get(self.cursor).map(|s| s.as_str())
        } else {
            None
        }
    }

    pub fn down(&mut self) -> Option<&str> {
        if self.cursor < self.entries.len() {
            self.cursor += 1;
            if self.cursor < self.entries.len() {
                self.entries.get(self.cursor).map(|s| s.as_str())
            } else {
                None
            }
        } else {
            None
        }
    }

    pub fn reset_cursor(&mut self) {
        self.cursor = self.entries.len();
    }

    fn from_entries(entries: Vec<String>) -> Self {
        let mut history = Self::new();
        for entry in entries.into_iter().take(MAX_HISTORY_ENTRIES) {
            if !entry.is_empty() && entry.len() <= MAX_HISTORY_ENTRY_BYTES {
                history.push(entry);
            }
        }
        history
    }
}

#[derive(Debug, Clone)]
pub struct InputStateStore {
    path: Option<PathBuf>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PersistedInputState {
    schema_version: u16,
    history: Vec<String>,
    draft: String,
}

impl InputStateStore {
    pub fn for_workspace(workspace: &WorkspacePolicy) -> Self {
        let root = std::env::var_os("ALETHEON_STATE_HOME")
            .map(PathBuf::from)
            .or_else(dirs::state_dir)
            .map(|path| path.join("aletheon").join("input"));
        let Some(root) = root else {
            return Self { path: None };
        };
        let principal = nix::unistd::Uid::current().as_raw();
        let mut hasher = Sha256::new();
        hasher.update(workspace.cwd().to_string_lossy().as_bytes());
        let digest = hasher.finalize();
        let digest = digest[..12]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        Self {
            path: Some(root.join(format!(
                "v{HISTORY_SCHEMA_VERSION}-uid{principal}-{digest}.json"
            ))),
        }
    }

    pub fn load(&self) -> (CommandHistory, String) {
        let Some(path) = &self.path else {
            return (CommandHistory::new(), String::new());
        };
        let Ok(bytes) = fs::read(path) else {
            return (CommandHistory::new(), String::new());
        };
        let Ok(state) = serde_json::from_slice::<PersistedInputState>(&bytes) else {
            return (CommandHistory::new(), String::new());
        };
        if state.schema_version != HISTORY_SCHEMA_VERSION {
            return (CommandHistory::new(), String::new());
        }
        let draft = if state.draft.len() <= MAX_DRAFT_BYTES {
            state.draft
        } else {
            String::new()
        };
        (CommandHistory::from_entries(state.history), draft)
    }

    pub fn save(&self, history: &CommandHistory, draft: &str) {
        let Some(path) = &self.path else {
            return;
        };
        if draft.len() > MAX_DRAFT_BYTES {
            return;
        }
        let Some(parent) = path.parent() else {
            return;
        };
        let state = PersistedInputState {
            schema_version: HISTORY_SCHEMA_VERSION,
            history: history.entries.clone(),
            draft: draft.to_owned(),
        };
        let Ok(bytes) = serde_json::to_vec(&state) else {
            return;
        };
        if fs::create_dir_all(parent).is_err() {
            return;
        }
        let temp = path.with_extension("json.tmp");
        if fs::write(&temp, bytes).is_ok() {
            let _ = fs::rename(temp, path);
        }
    }
}
