//! Append-only JSONL persistence for improvement proposals.
//!
//! Each line is one versioned proposal event. The store supports append,
//! rebuild on restart, and retrieval by ID.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use thiserror::Error;

use super::model::{ImprovementProposal, ProposalId};

#[derive(Debug, Error)]
pub enum ProposalStoreError {
    #[error("proposal with id {0} already exists")]
    AlreadyExists(ProposalId),
    #[error("proposal with id {0} not found")]
    NotFound(ProposalId),
    #[error("proposal persistence failed: {0}")]
    Persistence(String),
}

/// Append-only JSONL store for improvement proposals.
///
/// Proposals are serialized one per line. On open, all lines are read
/// to rebuild an in-memory index (deduplicated by ProposalId, last wins).
pub struct JsonlImprovementStore {
    path: Option<PathBuf>,
    proposals: Mutex<Vec<ImprovementProposal>>,
}

impl JsonlImprovementStore {
    /// Create an in-memory store with no file backing (for testing).
    pub fn in_memory() -> Self {
        Self {
            path: None,
            proposals: Mutex::new(Vec::new()),
        }
    }

    /// Open (or create) a JSONL file as the backing store.
    pub fn open(path: PathBuf) -> Result<Self, ProposalStoreError> {
        let proposals = if path.exists() {
            let file = std::fs::File::open(&path)
                .map_err(|e| ProposalStoreError::Persistence(e.to_string()))?;
            let reader = BufReader::new(file);
            let mut proposals: Vec<ImprovementProposal> = Vec::new();
            for line in reader.lines() {
                let line = line.map_err(|e| ProposalStoreError::Persistence(e.to_string()))?;
                if line.trim().is_empty() {
                    continue;
                }
                let proposal: ImprovementProposal = serde_json::from_str(&line)
                    .map_err(|e| ProposalStoreError::Persistence(e.to_string()))?;
                // Deduplicate: last write wins per id
                if let Some(existing) = proposals.iter_mut().find(|p| p.id == proposal.id) {
                    *existing = proposal;
                } else {
                    proposals.push(proposal);
                }
            }
            proposals
        } else {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| ProposalStoreError::Persistence(e.to_string()))?;
            }
            Vec::new()
        };
        Ok(Self {
            path: Some(path),
            proposals: Mutex::new(proposals),
        })
    }

    /// Append a proposal to the store.
    pub fn append(&self, proposal: ImprovementProposal) -> Result<(), ProposalStoreError> {
        let mut proposals = self
            .proposals
            .lock()
            .map_err(|e| ProposalStoreError::Persistence(format!("lock poisoned: {e}")))?;

        // Reject if the same ID already exists
        if proposals.iter().any(|p| p.id == proposal.id) {
            return Err(ProposalStoreError::AlreadyExists(proposal.id));
        }

        proposals.push(proposal);
        drop(proposals);
        self.persist()?;
        Ok(())
    }

    /// Update an existing proposal (e.g., after a state change).
    pub fn update(&self, proposal: ImprovementProposal) -> Result<(), ProposalStoreError> {
        let mut proposals = self
            .proposals
            .lock()
            .map_err(|e| ProposalStoreError::Persistence(format!("lock poisoned: {e}")))?;

        let existing = proposals
            .iter_mut()
            .find(|p| p.id == proposal.id)
            .ok_or_else(|| ProposalStoreError::NotFound(proposal.id.clone()))?;
        *existing = proposal;
        drop(proposals);
        self.persist()?;
        Ok(())
    }

    /// Get a proposal by ID.
    pub fn get(&self, id: &ProposalId) -> Result<Option<ImprovementProposal>, ProposalStoreError> {
        let proposals = self
            .proposals
            .lock()
            .map_err(|e| ProposalStoreError::Persistence(format!("lock poisoned: {e}")))?;
        Ok(proposals.iter().find(|p| &p.id == id).cloned())
    }

    /// List all accepted proposals.
    pub fn list_accepted(&self) -> Result<Vec<ImprovementProposal>, ProposalStoreError> {
        let proposals = self
            .proposals
            .lock()
            .map_err(|e| ProposalStoreError::Persistence(format!("lock poisoned: {e}")))?;
        Ok(proposals
            .iter()
            .filter(|p| matches!(p.state, super::model::ProposalState::Accepted))
            .cloned()
            .collect())
    }

    fn persist(&self) -> Result<(), ProposalStoreError> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let temp = path.with_extension(".tmp");
        let proposals = self
            .proposals
            .lock()
            .map_err(|e| ProposalStoreError::Persistence(format!("lock poisoned: {e}")))?;
        let mut file = std::fs::File::create(&temp)
            .map_err(|e| ProposalStoreError::Persistence(e.to_string()))?;
        for proposal in proposals.iter() {
            let line = serde_json::to_string(proposal)
                .map_err(|e| ProposalStoreError::Persistence(e.to_string()))?;
            writeln!(file, "{}", line)
                .map_err(|e| ProposalStoreError::Persistence(e.to_string()))?;
        }
        file.sync_all()
            .map_err(|e| ProposalStoreError::Persistence(e.to_string()))?;
        std::fs::rename(&temp, path).map_err(|e| ProposalStoreError::Persistence(e.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::model::ProposalState;
    use super::*;

    fn make_proposal(id: &str) -> ImprovementProposal {
        ImprovementProposal {
            id: ProposalId(id.to_string()),
            proposer: "test".to_string(),
            target_capability: "tool.config".to_string(),
            problem_ids: vec!["p1".to_string()],
            proposed_change: "test".to_string(),
            expected_benefit: "test".to_string(),
            possible_regressions: vec![],
            validation_plan: "sandbox".to_string(),
            rollback_plan: "revert".to_string(),
            authority_requirements: vec!["gov".to_string()],
            reversible: true,
            expires_at_ms: i64::MAX,
            state: ProposalState::Proposed,
        }
    }

    #[test]
    fn append_and_get() {
        let store = JsonlImprovementStore::in_memory();
        let proposal = make_proposal("prop-1");
        let id = proposal.id.clone();

        store.append(proposal).unwrap();
        let retrieved = store.get(&id).unwrap().unwrap();
        assert_eq!(retrieved.id, id);
    }

    #[test]
    fn duplicate_id_is_rejected() {
        let store = JsonlImprovementStore::in_memory();
        store.append(make_proposal("prop-1")).unwrap();
        let result = store.append(make_proposal("prop-1"));
        assert!(result.is_err());
    }

    #[test]
    fn update_existing_proposal() {
        let store = JsonlImprovementStore::in_memory();
        let mut proposal = make_proposal("prop-1");
        let id = proposal.id.clone();

        store.append(proposal.clone()).unwrap();

        // Update state
        proposal.state = ProposalState::Accepted;
        store.update(proposal).unwrap();

        let retrieved = store.get(&id).unwrap().unwrap();
        assert_eq!(retrieved.state, ProposalState::Accepted);
    }

    #[test]
    fn update_nonexistent_is_rejected() {
        let store = JsonlImprovementStore::in_memory();
        let result = store.update(make_proposal("prop-1"));
        assert!(result.is_err());
    }

    #[test]
    fn list_accepted_only() {
        let store = JsonlImprovementStore::in_memory();

        let mut p1 = make_proposal("prop-1");
        p1.state = ProposalState::Accepted;
        store.append(p1).unwrap();

        let mut p2 = make_proposal("prop-2");
        p2.state = ProposalState::Rejected;
        store.append(p2).unwrap();

        let accepted = store.list_accepted().unwrap();
        assert_eq!(accepted.len(), 1);
        assert_eq!(accepted[0].id, ProposalId("prop-1".into()));
    }

    #[test]
    fn reopening_rebuilds_index() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let path = dir.path().join("proposals.jsonl");

        // First session
        {
            let store = JsonlImprovementStore::open(path.clone()).unwrap();
            store.append(make_proposal("prop-1")).unwrap();
            store.append(make_proposal("prop-2")).unwrap();
        }

        // Second session — rebuilds from JSONL
        {
            let store = JsonlImprovementStore::open(path.clone()).unwrap();
            let p1 = store.get(&ProposalId("prop-1".into())).unwrap().unwrap();
            assert_eq!(p1.id, ProposalId("prop-1".into()));
            let p2 = store.get(&ProposalId("prop-2".into())).unwrap().unwrap();
            assert_eq!(p2.id, ProposalId("prop-2".into()));
        }
    }
}
