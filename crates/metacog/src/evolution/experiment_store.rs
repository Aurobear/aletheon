//! Evolution experiment persistence.
//!
//! Stores experiments and their lineage links as append-only JSONL.
//! Supports start/complete/get operations.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::Mutex;
use thiserror::Error;

use super::experiment::{EvolutionExperiment, ExperimentOutcome};
use super::lineage::LineageLink;

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum ExperimentStoreError {
    #[error("experiment already started with conflicting parameters")]
    Conflict,
    #[error("experiment not found: {0}")]
    NotFound(String),
    #[error("persistence failure: {0}")]
    Persistence(String),
}

// ---------------------------------------------------------------------------
// Store trait
// ---------------------------------------------------------------------------

/// Evolution experiment store — async port for tracking experiments
/// through their lifecycle.
#[async_trait]
pub trait ExperimentStore: Send + Sync {
    /// Begin a new experiment.  Fails with `Conflict` if an experiment
    /// with the same baseline/candidate pair is already active.
    async fn start_experiment(
        &self,
        experiment: EvolutionExperiment,
    ) -> Result<String, ExperimentStoreError>;

    /// Record the outcome of a previously started experiment.
    async fn complete_experiment(
        &self,
        experiment_id: &str,
        outcome: ExperimentOutcome,
    ) -> Result<(), ExperimentStoreError>;

    /// Retrieve an experiment by its id.
    async fn get_experiment(
        &self,
        experiment_id: &str,
    ) -> Result<Option<EvolutionExperiment>, ExperimentStoreError>;

    /// Retrieve the outcome of a completed experiment, if any.
    async fn get_outcome(
        &self,
        experiment_id: &str,
    ) -> Result<Option<ExperimentOutcome>, ExperimentStoreError>;

    /// Record a lineage link for an experiment.
    async fn record_lineage(
        &self,
        experiment_id: &str,
        link: LineageLink,
    ) -> Result<(), ExperimentStoreError>;

    /// Retrieve all lineage links for an experiment.
    async fn get_lineage(
        &self,
        experiment_id: &str,
    ) -> Result<Vec<LineageLink>, ExperimentStoreError>;
}

// ---------------------------------------------------------------------------
// JSONL-backed implementation
// ---------------------------------------------------------------------------

/// A single record in the experiment store.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
enum ExperimentRecord {
    #[serde(rename = "experiment_started")]
    Started {
        experiment_id: String,
        experiment: EvolutionExperiment,
    },
    #[serde(rename = "experiment_completed")]
    Completed {
        experiment_id: String,
        outcome: ExperimentOutcome,
    },
    #[serde(rename = "lineage_link")]
    Lineage {
        experiment_id: String,
        link: LineageLink,
    },
}

/// JSONL-backed experiment store.
///
/// Each line is one versioned event in the store.  The in-memory index
/// is rebuilt from the file on open.
pub struct JsonlExperimentStore {
    path: Option<PathBuf>,
    /// In-memory index of records.
    records: Mutex<Vec<ExperimentRecord>>,
}

impl JsonlExperimentStore {
    /// Create an in-memory-only store (for tests).
    pub fn in_memory() -> Self {
        Self {
            path: None,
            records: Mutex::new(Vec::new()),
        }
    }

    /// Open or create a JSONL file as the backing store.
    pub fn open(path: PathBuf) -> Result<Self, ExperimentStoreError> {
        let records = if path.exists() {
            let file = std::fs::File::open(&path)
                .map_err(|e| ExperimentStoreError::Persistence(e.to_string()))?;
            let reader = BufReader::new(file);
            let mut records = Vec::new();
            for line in reader.lines() {
                let line = line.map_err(|e| ExperimentStoreError::Persistence(e.to_string()))?;
                if line.trim().is_empty() {
                    continue;
                }
                let record: ExperimentRecord = serde_json::from_str(&line)
                    .map_err(|e| ExperimentStoreError::Persistence(e.to_string()))?;
                records.push(record);
            }
            records
        } else {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| ExperimentStoreError::Persistence(e.to_string()))?;
            }
            Vec::new()
        };
        Ok(Self {
            path: Some(path),
            records: Mutex::new(records),
        })
    }

    /// Full rewrite of the backing file (atomic via temp-file rename).
    fn persist(&self) -> Result<(), ExperimentStoreError> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let temp = path.with_extension(".tmp");
        let records = self
            .records
            .lock()
            .map_err(|e| ExperimentStoreError::Persistence(format!("lock poisoned: {}", e)))?;
        let mut file = std::fs::File::create(&temp)
            .map_err(|e| ExperimentStoreError::Persistence(e.to_string()))?;
        for record in records.iter() {
            let line = serde_json::to_string(record)
                .map_err(|e| ExperimentStoreError::Persistence(e.to_string()))?;
            writeln!(file, "{}", line)
                .map_err(|e| ExperimentStoreError::Persistence(e.to_string()))?;
        }
        file.sync_all()
            .map_err(|e| ExperimentStoreError::Persistence(e.to_string()))?;
        std::fs::rename(&temp, path)
            .map_err(|e| ExperimentStoreError::Persistence(e.to_string()))?;
        Ok(())
    }
}

#[async_trait]
impl ExperimentStore for JsonlExperimentStore {
    async fn start_experiment(
        &self,
        experiment: EvolutionExperiment,
    ) -> Result<String, ExperimentStoreError> {
        let experiment_id = format!(
            "{}-vs-{}",
            experiment.baseline_version, experiment.candidate_version
        );

        let mut records = self
            .records
            .lock()
            .map_err(|e| ExperimentStoreError::Persistence(format!("lock poisoned: {}", e)))?;

        // Check for existing active experiment with same pair
        for r in records.iter() {
            if let ExperimentRecord::Started {
                experiment_id: eid, ..
            } = r
            {
                if eid == &experiment_id {
                    return Err(ExperimentStoreError::Conflict);
                }
            }
        }

        records.push(ExperimentRecord::Started {
            experiment_id: experiment_id.clone(),
            experiment,
        });
        drop(records);
        self.persist()?;
        Ok(experiment_id)
    }

    async fn complete_experiment(
        &self,
        experiment_id: &str,
        outcome: ExperimentOutcome,
    ) -> Result<(), ExperimentStoreError> {
        let mut records = self
            .records
            .lock()
            .map_err(|e| ExperimentStoreError::Persistence(format!("lock poisoned: {}", e)))?;

        // Verify the experiment was started
        let started = records.iter().any(|r| {
            matches!(r,
                ExperimentRecord::Started { experiment_id: eid, .. } if eid == experiment_id)
        });
        if !started {
            return Err(ExperimentStoreError::NotFound(experiment_id.to_string()));
        }

        records.push(ExperimentRecord::Completed {
            experiment_id: experiment_id.to_string(),
            outcome,
        });
        drop(records);
        self.persist()?;
        Ok(())
    }

    async fn get_experiment(
        &self,
        experiment_id: &str,
    ) -> Result<Option<EvolutionExperiment>, ExperimentStoreError> {
        let records = self
            .records
            .lock()
            .map_err(|e| ExperimentStoreError::Persistence(format!("lock poisoned: {}", e)))?;
        for r in records.iter() {
            if let ExperimentRecord::Started {
                experiment_id: eid,
                experiment,
            } = r
            {
                if eid == experiment_id {
                    return Ok(Some(experiment.clone()));
                }
            }
        }
        Ok(None)
    }

    async fn get_outcome(
        &self,
        experiment_id: &str,
    ) -> Result<Option<ExperimentOutcome>, ExperimentStoreError> {
        let records = self
            .records
            .lock()
            .map_err(|e| ExperimentStoreError::Persistence(format!("lock poisoned: {}", e)))?;
        for r in records.iter() {
            if let ExperimentRecord::Completed {
                experiment_id: eid,
                outcome,
            } = r
            {
                if eid == experiment_id {
                    return Ok(Some(outcome.clone()));
                }
            }
        }
        Ok(None)
    }

    async fn record_lineage(
        &self,
        experiment_id: &str,
        link: LineageLink,
    ) -> Result<(), ExperimentStoreError> {
        let mut records = self
            .records
            .lock()
            .map_err(|e| ExperimentStoreError::Persistence(format!("lock poisoned: {}", e)))?;
        records.push(ExperimentRecord::Lineage {
            experiment_id: experiment_id.to_string(),
            link,
        });
        drop(records);
        self.persist()?;
        Ok(())
    }

    async fn get_lineage(
        &self,
        experiment_id: &str,
    ) -> Result<Vec<LineageLink>, ExperimentStoreError> {
        let records = self
            .records
            .lock()
            .map_err(|e| ExperimentStoreError::Persistence(format!("lock poisoned: {}", e)))?;
        Ok(records
            .iter()
            .filter_map(|r| {
                if let ExperimentRecord::Lineage {
                    experiment_id: eid,
                    link,
                } = r
                {
                    if eid == experiment_id {
                        return Some(link.clone());
                    }
                }
                None
            })
            .collect())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evolution::experiment::{
        EvaluationReport, ExperimentDecision, GateResult, ProblemRecord, ProblemSeverity,
        ProblemState,
    };

    fn make_experiment() -> EvolutionExperiment {
        EvolutionExperiment {
            baseline_version: "1.0.0".into(),
            candidate_version: "1.1.0".into(),
            target_problem_ids: vec!["p1".into()],
            baseline_score_distribution: vec![80.0, 85.0],
            success_threshold: 5_000,
            rollback_threshold: 3_000,
            observation_window_ms: 60_000,
        }
    }

    fn make_outcome(decision: ExperimentDecision) -> ExperimentOutcome {
        ExperimentOutcome {
            pre_reports: vec![],
            post_reports: vec![],
            regressions: vec![],
            new_problems: vec![],
            decision,
        }
    }

    fn make_link(experiment_id: &str) -> LineageLink {
        LineageLink::new(
            "problem-1".into(),
            "proposal-1".into(),
            "mutation-1".into(),
            "candidate-1".into(),
            "approval-1".into(),
            format!("hash-of-{}", experiment_id),
            "outcome-1".into(),
        )
    }

    #[tokio::test]
    async fn start_and_get_experiment() {
        let store = JsonlExperimentStore::in_memory();
        let exp = make_experiment();
        let id = store.start_experiment(exp.clone()).await.unwrap();
        assert_eq!(id, "1.0.0-vs-1.1.0");

        let got = store.get_experiment(&id).await.unwrap().unwrap();
        assert_eq!(got.baseline_version, exp.baseline_version);
        assert_eq!(got.candidate_version, exp.candidate_version);
    }

    #[tokio::test]
    async fn complete_and_read_outcome() {
        let store = JsonlExperimentStore::in_memory();
        let exp = make_experiment();
        let id = store.start_experiment(exp).await.unwrap();

        let outcome = make_outcome(ExperimentDecision::Promote);
        store
            .complete_experiment(&id, outcome.clone())
            .await
            .unwrap();

        let got = store.get_outcome(&id).await.unwrap().unwrap();
        assert_eq!(got.decision, ExperimentDecision::Promote);
    }

    #[tokio::test]
    async fn duplicate_start_is_conflict() {
        let store = JsonlExperimentStore::in_memory();
        store.start_experiment(make_experiment()).await.unwrap();
        let result = store.start_experiment(make_experiment()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn complete_nonexistent_returns_not_found() {
        let store = JsonlExperimentStore::in_memory();
        let outcome = make_outcome(ExperimentDecision::Promote);
        let result = store.complete_experiment("nonexistent", outcome).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn record_and_read_lineage() {
        let store = JsonlExperimentStore::in_memory();
        let id = store.start_experiment(make_experiment()).await.unwrap();

        let link = make_link(&id);
        store.record_lineage(&id, link.clone()).await.unwrap();

        let links = store.get_lineage(&id).await.unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].problem_id, "problem-1");
        assert_eq!(links[0].proposal_id, "proposal-1");
    }

    #[tokio::test]
    async fn reopen_jsonl_file_rebuilds_index() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().with_extension("jsonl");

        let id;
        {
            let store = JsonlExperimentStore::open(path.clone()).unwrap();
            let exp = make_experiment();
            id = store.start_experiment(exp).await.unwrap();

            let link = make_link(&id);
            store.record_lineage(&id, link).await.unwrap();

            store
                .complete_experiment(&id, make_outcome(ExperimentDecision::Rollback))
                .await
                .unwrap();
        }

        // Reopen — verify all data is still there
        {
            let store = JsonlExperimentStore::open(path.clone()).unwrap();

            let exp = store.get_experiment(&id).await.unwrap().unwrap();
            assert_eq!(exp.baseline_version, "1.0.0");

            let outcome = store.get_outcome(&id).await.unwrap().unwrap();
            assert_eq!(outcome.decision, ExperimentDecision::Rollback);

            let links = store.get_lineage(&id).await.unwrap();
            assert_eq!(links.len(), 1);
            assert_eq!(links[0].problem_id, "problem-1");
        }
    }
}
