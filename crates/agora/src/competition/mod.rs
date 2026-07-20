//! Bounded, deterministic competition for typed Agora candidates.

use fabric::{
    AgoraSpaceId, CandidateScore, ContentId, MonoTime, ProcessId, SalienceVector,
    SelectionExplanation, SelectionResult, WorkspaceCandidate,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, VecDeque};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectionPolicy {
    pub version: u16,
    pub weights: SalienceVector,
    pub aging_per_ms: f64,
    pub unresolved_dependency_boost: f64,
    pub repetition_penalty: f64,
    pub refractory_penalty: f64,
    pub ignition_threshold: f64,
    pub max_consecutive_source_wins: usize,
}

impl SelectionPolicy {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.version > 0,
            "selection policy version must be non-zero"
        );
        self.weights.validate()?;
        for (name, value) in [
            ("aging_per_ms", self.aging_per_ms),
            (
                "unresolved_dependency_boost",
                self.unresolved_dependency_boost,
            ),
            ("repetition_penalty", self.repetition_penalty),
            ("refractory_penalty", self.refractory_penalty),
            ("ignition_threshold", self.ignition_threshold),
        ] {
            anyhow::ensure!(
                value.is_finite() && value >= 0.0,
                "{name} must be finite and non-negative"
            );
        }
        anyhow::ensure!(
            self.max_consecutive_source_wins > 0,
            "source win limit must be non-zero"
        );
        Ok(())
    }
}

impl Default for SelectionPolicy {
    fn default() -> Self {
        Self {
            version: 1,
            weights: SalienceVector {
                urgency: 1.0,
                goal_relevance: 1.0,
                self_relevance: 1.0,
                novelty: 1.0,
                confidence: 1.0,
                prediction_error: 1.0,
                affect_intensity: 1.0,
                social_relevance: 1.0,
            },
            aging_per_ms: 0.000_001,
            unresolved_dependency_boost: 0.1,
            repetition_penalty: 0.25,
            refractory_penalty: 10.0,
            ignition_threshold: 0.5,
            max_consecutive_source_wins: 2,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidatePoolConfig {
    pub capacity: usize,
    pub per_source_capacity: usize,
    pub max_coalition: usize,
    pub policy: SelectionPolicy,
}

impl CandidatePoolConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            (1..=4096).contains(&self.capacity),
            "candidate capacity is invalid"
        );
        anyhow::ensure!(
            (1..=self.capacity).contains(&self.per_source_capacity),
            "per-source capacity is invalid"
        );
        anyhow::ensure!(
            (1..=32).contains(&self.max_coalition),
            "coalition limit is invalid"
        );
        anyhow::ensure!(
            self.max_coalition <= self.capacity,
            "coalition cannot exceed candidate capacity"
        );
        self.policy.validate()
    }
}

impl Default for CandidatePoolConfig {
    fn default() -> Self {
        Self {
            capacity: 256,
            per_source_capacity: 64,
            max_coalition: 4,
            policy: SelectionPolicy::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdmissionOutcome {
    Accepted { id: ContentId },
    Duplicate { existing: ContentId },
    RejectedCapacity,
    RejectedSourceQuota { source: ProcessId },
    RejectedWrongSpace,
    RejectedInvalid { reason: String },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdmissionMetrics {
    pub accepted: u64,
    pub duplicates: u64,
    pub expired: u64,
    pub capacity_rejections: u64,
    pub source_quota_rejections: u64,
    pub invalid_rejections: u64,
    pub wrong_space_rejections: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionMetrics {
    pub cycles: u64,
    pub ignitions: u64,
    pub below_threshold: u64,
    pub refractory_applications: u64,
    pub aging_applications: u64,
    pub selections_by_source: HashMap<ProcessId, u64>,
}

pub struct CandidatePool {
    space: AgoraSpaceId,
    config: CandidatePoolConfig,
    candidates: BTreeMap<ContentId, WorkspaceCandidate>,
    fingerprints: HashMap<String, ContentId>,
    selected_fingerprints: HashMap<String, u32>,
    recent_sources: VecDeque<ProcessId>,
    admission_metrics: AdmissionMetrics,
    selection_metrics: SelectionMetrics,
}

impl CandidatePool {
    pub fn new(space: AgoraSpaceId, config: CandidatePoolConfig) -> anyhow::Result<Self> {
        anyhow::ensure!(!space.0.trim().is_empty(), "candidate pool space is empty");
        config.validate()?;
        Ok(Self {
            space,
            config,
            candidates: BTreeMap::new(),
            fingerprints: HashMap::new(),
            selected_fingerprints: HashMap::new(),
            recent_sources: VecDeque::new(),
            admission_metrics: AdmissionMetrics::default(),
            selection_metrics: SelectionMetrics::default(),
        })
    }

    pub fn admit(&mut self, candidate: WorkspaceCandidate, now: MonoTime) -> AdmissionOutcome {
        self.expire(now);
        if candidate.space != self.space {
            self.admission_metrics.wrong_space_rejections += 1;
            return AdmissionOutcome::RejectedWrongSpace;
        }
        if let Err(error) = candidate.validate() {
            self.admission_metrics.invalid_rejections += 1;
            return AdmissionOutcome::RejectedInvalid {
                reason: error.to_string(),
            };
        }
        if candidate.is_expired_at(now) {
            self.admission_metrics.invalid_rejections += 1;
            return AdmissionOutcome::RejectedInvalid {
                reason: "candidate already expired".into(),
            };
        }
        let fingerprint = match candidate.content_fingerprint() {
            Ok(value) => value,
            Err(error) => {
                self.admission_metrics.invalid_rejections += 1;
                return AdmissionOutcome::RejectedInvalid {
                    reason: error.to_string(),
                };
            }
        };
        if let Some(existing) = self.fingerprints.get(&fingerprint) {
            self.admission_metrics.duplicates += 1;
            return AdmissionOutcome::Duplicate {
                existing: *existing,
            };
        }
        if self
            .candidates
            .values()
            .filter(|current| current.source == candidate.source)
            .count()
            >= self.config.per_source_capacity
        {
            self.admission_metrics.source_quota_rejections += 1;
            return AdmissionOutcome::RejectedSourceQuota {
                source: candidate.source,
            };
        }
        if self.candidates.len() >= self.config.capacity {
            self.admission_metrics.capacity_rejections += 1;
            return AdmissionOutcome::RejectedCapacity;
        }
        let id = candidate.id;
        if self.candidates.contains_key(&id) {
            self.admission_metrics.invalid_rejections += 1;
            return AdmissionOutcome::RejectedInvalid {
                reason: "candidate id already exists".into(),
            };
        }
        self.fingerprints.insert(fingerprint, id);
        self.candidates.insert(id, candidate);
        self.admission_metrics.accepted += 1;
        AdmissionOutcome::Accepted { id }
    }

    pub fn select(&mut self, now: MonoTime) -> SelectionResult {
        self.expire(now);
        self.selection_metrics.cycles += 1;
        let alternative_sources = self
            .candidates
            .values()
            .map(|candidate| candidate.source)
            .collect::<std::collections::HashSet<_>>()
            .len()
            > 1;
        let mut scored: Vec<(CandidateScore, MonoTime)> = self
            .candidates
            .values()
            .map(|candidate| {
                let fingerprint = candidate
                    .content_fingerprint()
                    .expect("validated candidate fingerprint remains serializable");
                let salience = dot(candidate.salience, self.config.policy.weights);
                let aging_boost = now.0.saturating_sub(candidate.created_at.0) as f64
                    * self.config.policy.aging_per_ms;
                let dependency_boost = candidate
                    .dependencies
                    .iter()
                    .filter(|dependency| self.candidates.contains_key(dependency))
                    .count() as f64
                    * self.config.policy.unresolved_dependency_boost;
                let repetition_penalty = self
                    .selected_fingerprints
                    .get(&fingerprint)
                    .copied()
                    .unwrap_or(0) as f64
                    * self.config.policy.repetition_penalty;
                let refractory = alternative_sources
                    && self.consecutive_wins(candidate.source)
                        >= self.config.policy.max_consecutive_source_wins;
                let refractory_penalty = if refractory {
                    self.config.policy.refractory_penalty
                } else {
                    0.0
                };
                (
                    CandidateScore {
                        id: candidate.id,
                        source: candidate.source,
                        salience,
                        aging_boost,
                        dependency_boost,
                        repetition_penalty,
                        refractory_penalty,
                        total: salience + aging_boost + dependency_boost
                            - repetition_penalty
                            - refractory_penalty,
                    },
                    candidate.created_at,
                )
            })
            .collect();
        scored.sort_by(|(left, left_created), (right, right_created)| {
            right
                .total
                .total_cmp(&left.total)
                .then_with(|| left_created.cmp(right_created))
                .then_with(|| left.id.cmp(&right.id))
        });
        let rejected_below_ignition = scored
            .iter()
            .filter(|(score, _)| score.total < self.config.policy.ignition_threshold)
            .map(|(score, _)| score.id)
            .collect();
        let mut selected_ids = Vec::new();
        if let Some((winner, _)) = scored
            .first()
            .filter(|(score, _)| score.total >= self.config.policy.ignition_threshold)
        {
            selected_ids.push(winner.id);
            if let Some(candidate) = self.candidates.get(&winner.id) {
                let mut dependencies: Vec<_> = candidate
                    .dependencies
                    .iter()
                    .copied()
                    .filter(|id| self.candidates.contains_key(id))
                    .collect();
                dependencies.sort();
                dependencies.dedup();
                selected_ids.extend(
                    dependencies
                        .into_iter()
                        .take(self.config.max_coalition.saturating_sub(1)),
                );
            }
        }
        let selected = selected_ids
            .iter()
            .filter_map(|id| self.candidates.get(id).cloned())
            .collect();
        SelectionResult {
            selected,
            explanation: SelectionExplanation {
                policy_version: self.config.policy.version,
                evaluated: scored.into_iter().map(|(score, _)| score).collect(),
                selected_ids,
                rejected_below_ignition,
            },
        }
    }

    pub fn finalize_selection(&mut self, result: &SelectionResult) -> anyhow::Result<()> {
        self.validate_selection(result)?;
        let primary_source = result.selected[0].source;
        for candidate in &result.selected {
            let fingerprint = candidate.content_fingerprint()?;
            *self
                .selected_fingerprints
                .entry(fingerprint.clone())
                .or_default() += 1;
            self.fingerprints.remove(&fingerprint);
            self.candidates.remove(&candidate.id);
        }
        let refractory_count = result
            .explanation
            .evaluated
            .iter()
            .filter(|score| score.refractory_penalty > 0.0)
            .count() as u64;
        let aging_count = result
            .explanation
            .evaluated
            .iter()
            .filter(|score| score.aging_boost > 0.0)
            .count() as u64;
        self.selection_metrics.refractory_applications += refractory_count;
        self.selection_metrics.aging_applications += aging_count;
        self.selection_metrics.ignitions += 1;
        *self
            .selection_metrics
            .selections_by_source
            .entry(primary_source)
            .or_default() += 1;
        self.recent_sources.push_back(primary_source);
        while self.recent_sources.len() > self.config.policy.max_consecutive_source_wins {
            self.recent_sources.pop_front();
        }
        Ok(())
    }

    pub fn validate_selection(&self, result: &SelectionResult) -> anyhow::Result<()> {
        anyhow::ensure!(
            !result.selected.is_empty(),
            "cannot finalize an empty selection"
        );
        anyhow::ensure!(
            result.explanation.selected_ids
                == result
                    .selected
                    .iter()
                    .map(|candidate| candidate.id)
                    .collect::<Vec<_>>(),
            "selection result IDs do not match candidates"
        );
        for candidate in &result.selected {
            let stored = self
                .candidates
                .get(&candidate.id)
                .ok_or_else(|| anyhow::anyhow!("selected candidate is no longer pending"))?;
            anyhow::ensure!(
                stored.content_fingerprint()? == candidate.content_fingerprint()?,
                "selected candidate changed"
            );
        }
        Ok(())
    }

    pub fn record_no_ignition(&mut self, result: &SelectionResult) -> anyhow::Result<()> {
        anyhow::ensure!(result.selected.is_empty(), "selection already ignited");
        self.selection_metrics.below_threshold += 1;
        Ok(())
    }

    pub fn pending(&self) -> Vec<WorkspaceCandidate> {
        self.candidates.values().cloned().collect()
    }

    /// Replace only the mutable salience projection for a still-pending
    /// candidate. Content identity, provenance and deduplication fingerprints
    /// remain unchanged.
    pub fn update_salience(
        &mut self,
        id: ContentId,
        salience: SalienceVector,
    ) -> anyhow::Result<()> {
        salience.validate()?;
        let candidate = self
            .candidates
            .get_mut(&id)
            .ok_or_else(|| anyhow::anyhow!("candidate is no longer pending"))?;
        candidate.salience = salience;
        Ok(())
    }
    pub fn len(&self) -> usize {
        self.candidates.len()
    }
    pub fn is_empty(&self) -> bool {
        self.candidates.is_empty()
    }
    pub fn admission_metrics(&self) -> &AdmissionMetrics {
        &self.admission_metrics
    }
    pub fn selection_metrics(&self) -> &SelectionMetrics {
        &self.selection_metrics
    }

    fn consecutive_wins(&self, source: ProcessId) -> usize {
        self.recent_sources
            .iter()
            .rev()
            .take_while(|winner| **winner == source)
            .count()
    }
    fn expire(&mut self, now: MonoTime) {
        let expired: Vec<_> = self
            .candidates
            .iter()
            .filter(|(_, candidate)| candidate.is_expired_at(now))
            .map(|(id, _)| *id)
            .collect();
        for id in expired {
            if let Some(candidate) = self.candidates.remove(&id) {
                if let Ok(fingerprint) = candidate.content_fingerprint() {
                    self.fingerprints.remove(&fingerprint);
                }
                self.admission_metrics.expired += 1;
            }
        }
    }
}

fn dot(values: SalienceVector, weights: SalienceVector) -> f64 {
    values
        .values()
        .into_iter()
        .zip(weights.values())
        .map(|(value, weight)| f64::from(value) * f64::from(weight))
        .sum()
}
