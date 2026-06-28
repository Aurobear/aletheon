//! Checkpoint and rewind system.
//!
//! Provides snapshot-based rewind for file edits.
//! One checkpoint per user turn. Uses Previewer trait to capture state.

use base::tool::FileSnap;
use std::path::Path;
use std::time::SystemTime;
use tracing::info;

/// Scope of rewind operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RewindScope {
    Code,
    Conversation,
    Both,
}

/// One checkpoint per user turn.
#[derive(Debug, Clone)]
pub struct Checkpoint {
    pub turn: usize,
    pub time: SystemTime,
    pub prompt: String,
    pub msg_index: usize,
    pub files: Vec<FileSnap>,
}

/// Checkpoint store for a session.
pub struct CheckpointStore {
    #[allow(dead_code)]
    session_dir: std::path::PathBuf,
    checkpoints: Vec<Checkpoint>,
    /// Currently open checkpoint (not yet sealed).
    current: Option<Checkpoint>,
}

impl CheckpointStore {
    pub fn new(session_dir: &Path) -> Self {
        Self {
            session_dir: session_dir.to_path_buf(),
            checkpoints: Vec::new(),
            current: None,
        }
    }

    /// Open a new checkpoint for the current turn.
    pub fn open_checkpoint(
        &mut self,
        turn: usize,
        prompt: &str,
        msg_index: usize,
    ) -> std::io::Result<()> {
        self.current = Some(Checkpoint {
            turn,
            time: SystemTime::now(),
            prompt: prompt.to_string(),
            msg_index,
            files: Vec::new(),
        });
        Ok(())
    }

    /// Add a file snapshot to the current open checkpoint.
    pub fn add_snap(&mut self, snap: FileSnap) {
        if let Some(ref mut cp) = self.current {
            // Dedup: only keep first snapshot per path per turn
            if !cp.files.iter().any(|s| s.path == snap.path) {
                cp.files.push(snap);
            }
        }
    }

    /// Seal the current checkpoint (move to completed list).
    pub fn seal_checkpoint(&mut self) {
        if let Some(cp) = self.current.take() {
            info!(turn = cp.turn, files = cp.files.len(), "Checkpoint sealed");
            self.checkpoints.push(cp);
        }
    }

    /// Get all sealed checkpoints.
    pub fn checkpoints(&self) -> &[Checkpoint] {
        &self.checkpoints
    }

    /// Get a checkpoint by turn number.
    pub fn get_checkpoint(&self, turn: usize) -> Option<&Checkpoint> {
        self.checkpoints.iter().find(|cp| cp.turn == turn)
    }

    /// Rewind file state to a checkpoint.
    pub fn rewind_code(&self, turn: usize) -> std::io::Result<()> {
        let cp = self
            .get_checkpoint(turn)
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "checkpoint"))?;

        for snap in &cp.files {
            if let Err(e) = snap.restore() {
                tracing::warn!(path = %snap.path.display(), error = %e, "Failed to restore snapshot");
            } else {
                info!(path = %snap.path.display(), "Restored snapshot");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    #[test]
    fn checkpoint_captures_file_snap() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("test.txt");
        fs::write(&file, "original").unwrap();

        let snap = FileSnap::capture(&file).unwrap();
        assert_eq!(snap.path, file);
        assert_eq!(snap.content.as_deref(), Some("original"));
    }

    #[test]
    fn checkpoint_captures_nonexistent_file() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("missing.txt");

        let snap = FileSnap::capture(&file).unwrap();
        assert!(snap.content.is_none());
    }

    #[test]
    fn checkpoint_store_and_list() {
        let tmp = TempDir::new().unwrap();
        let mut store = CheckpointStore::new(tmp.path());

        store.open_checkpoint(1, "first turn", 0).unwrap();
        store.add_snap(FileSnap {
            path: PathBuf::from("/tmp/test.txt"),
            content: Some("content".into()),
        });
        store.seal_checkpoint();

        store.open_checkpoint(2, "second turn", 5).unwrap();
        store.seal_checkpoint();

        assert_eq!(store.checkpoints().len(), 2);
        assert_eq!(store.checkpoints()[0].turn, 1);
        assert_eq!(store.checkpoints()[1].turn, 2);
    }

    #[test]
    fn rewind_code_restores_files() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("test.txt");
        fs::write(&file, "original").unwrap();

        let mut store = CheckpointStore::new(tmp.path());
        store.open_checkpoint(1, "turn 1", 0).unwrap();
        store.add_snap(FileSnap::capture(&file).unwrap());
        store.seal_checkpoint();

        // Modify file
        fs::write(&file, "modified").unwrap();

        // Rewind
        store.rewind_code(1).unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "original");
    }

    #[test]
    fn rewind_code_deletes_created_file() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("new.txt");

        let mut store = CheckpointStore::new(tmp.path());
        store.open_checkpoint(1, "turn 1", 0).unwrap();
        store.add_snap(FileSnap::capture(&file).unwrap()); // None content
        store.seal_checkpoint();

        // Create file
        fs::write(&file, "created").unwrap();

        // Rewind should delete it
        store.rewind_code(1).unwrap();
        assert!(!file.exists());
    }

    #[test]
    fn get_checkpoint_by_turn() {
        let tmp = TempDir::new().unwrap();
        let mut store = CheckpointStore::new(tmp.path());

        store.open_checkpoint(5, "turn 5", 10).unwrap();
        store.seal_checkpoint();

        assert!(store.get_checkpoint(5).is_some());
        assert!(store.get_checkpoint(3).is_none());
    }
}
