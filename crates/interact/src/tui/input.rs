use std::{fs, io::Write, path::PathBuf};

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};

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

    pub fn entries(&self) -> &[String] {
        &self.entries
    }

    fn from_entries(entries: Vec<String>) -> Self {
        let mut history = Self::new();
        for entry in entries.into_iter().take(MAX_HISTORY_ENTRIES) {
            if history_entry_allowed(&entry) {
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
        let principal = nix::unistd::Uid::effective().as_raw();
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
        if !private_regular_file(path) {
            return (CommandHistory::new(), String::new());
        }
        let Ok(bytes) = fs::read(path) else {
            return (CommandHistory::new(), String::new());
        };
        let Ok(state) = serde_json::from_slice::<PersistedInputState>(&bytes) else {
            return (CommandHistory::new(), String::new());
        };
        if state.schema_version != HISTORY_SCHEMA_VERSION {
            return (CommandHistory::new(), String::new());
        }
        let draft = if state.draft.len() <= MAX_DRAFT_BYTES && !looks_sensitive(&state.draft) {
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
            history: history
                .entries
                .iter()
                .filter(|entry| history_entry_allowed(entry))
                .cloned()
                .collect(),
            draft: if looks_sensitive(draft) {
                String::new()
            } else {
                draft.to_owned()
            },
        };
        let Ok(bytes) = serde_json::to_vec(&state) else {
            return;
        };
        if fs::create_dir_all(parent).is_err() || !prepare_private_directory(parent) {
            return;
        }
        if path.exists() && !private_regular_file(path) {
            return;
        }
        let temp = path.with_extension(format!("{}.tmp", std::process::id()));
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        if let Ok(mut file) = options.open(&temp) {
            if file.write_all(&bytes).is_ok() && file.sync_all().is_ok() {
                let _ = fs::rename(&temp, path);
            }
            let _ = fs::remove_file(temp);
        }
    }

    pub fn delete(&self) {
        if let Some(path) = &self.path {
            if private_regular_file(path) {
                let _ = fs::remove_file(path);
            }
        }
    }

    #[cfg(test)]
    fn at_path(path: PathBuf) -> Self {
        Self { path: Some(path) }
    }
}

fn history_entry_allowed(entry: &str) -> bool {
    !entry.is_empty() && entry.len() <= MAX_HISTORY_ENTRY_BYTES && !looks_sensitive(entry)
}

fn looks_sensitive(value: &str) -> bool {
    let value = value.to_ascii_lowercase();
    ["api_key=", "apikey=", "token=", "password=", "secret="]
        .iter()
        .any(|marker| value.contains(marker))
}

#[cfg(unix)]
fn prepare_private_directory(path: &std::path::Path) -> bool {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata.uid() != nix::unistd::Uid::effective().as_raw()
    {
        return false;
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).is_ok()
}

#[cfg(not(unix))]
fn prepare_private_directory(_path: &std::path::Path) -> bool {
    false
}

#[cfg(unix)]
fn private_regular_file(path: &std::path::Path) -> bool {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    metadata.is_file()
        && !metadata.file_type().is_symlink()
        && metadata.uid() == nix::unistd::Uid::effective().as_raw()
        && metadata.mode() & 0o077 == 0
}

#[cfg(not(unix))]
fn private_regular_file(_path: &std::path::Path) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn u_input_002_history_and_draft_survive_store_reconstruction() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("input").join("state.json");
        let store = InputStateStore::at_path(path.clone());
        let mut history = CommandHistory::new();
        history.push("first prompt".into());
        store.save(&history, "unfinished draft");

        let restarted = InputStateStore::at_path(path);
        let (mut loaded, draft) = restarted.load();
        assert_eq!(draft, "unfinished draft");
        assert_eq!(loaded.up(), Some("first prompt"));
        restarted.delete();
        assert_eq!(restarted.load().1, "");
    }

    #[test]
    fn sensitive_entries_are_redacted_from_persistence() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("input").join("state.json");
        let store = InputStateStore::at_path(path);
        let mut history = CommandHistory::new();
        history.push("token=do-not-store".into());
        store.save(&history, "password=also-private");
        let (mut loaded, draft) = store.load();
        assert!(loaded.up().is_none());
        assert!(draft.is_empty());
    }
}
