use super::repository::ConsolidatedRecord;
use super::{ConsolidationRepository, MemoryCandidate};
use crate::{CandidateDecisionLabel, MemoryAuthority, MemoryRecordId, MemoryScope};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsolidationDecision {
    Insert,
    Merge,
    Reject,
    Supersede,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsolidationOutcome {
    pub consumed: usize,
    pub decisions: Vec<(i64, ConsolidationDecision, Option<MemoryRecordId>)>,
}
pub struct ScopedConsolidator<'a> {
    repository: &'a ConsolidationRepository,
    max_candidates: usize,
    lease_ms: u64,
}
impl<'a> ScopedConsolidator<'a> {
    pub fn new(repository: &'a ConsolidationRepository) -> Self {
        Self {
            repository,
            max_candidates: 64,
            lease_ms: 60_000,
        }
    }
    pub fn run(
        &self,
        scope: &MemoryScope,
        owner: &str,
        now_ms: u64,
        approval_evidence: Option<&str>,
    ) -> anyhow::Result<ConsolidationOutcome> {
        let Some(lease) = self
            .repository
            .acquire_scope(scope, owner, now_ms, self.lease_ms)?
        else {
            return Ok(ConsolidationOutcome {
                consumed: 0,
                decisions: vec![],
            });
        };
        let candidates = self
            .repository
            .pending_candidates(scope, self.max_candidates)?;
        let current = self.repository.current_records(scope)?;
        let mut seen = current
            .iter()
            .map(|record| (normalize(&record.content), record.id.0.clone()))
            .collect::<HashMap<_, _>>();
        let current_by_lineage = current
            .into_iter()
            .map(|record| (lineage_key(&record.kind, &record.source_event_ids), record))
            .collect::<HashMap<_, _>>();
        let mut decisions = Vec::new();
        for (id, candidate) in candidates {
            let (decision, record) = decide(
                scope,
                &candidate,
                approval_evidence,
                &mut seen,
                &current_by_lineage,
            );
            decisions.push((id, decision, record));
        }
        let watermark = watermark(&decisions);
        let rows = decisions
            .iter()
            .map(|(id, d, r)| {
                (
                    *id,
                    serde_json::to_string(d).unwrap(),
                    r.as_ref().map(|v| v.0.clone()),
                )
            })
            .collect::<Vec<_>>();
        self.repository
            .commit_decisions(&lease, &watermark, &rows, now_ms)?;
        let metrics = self.repository.metrics();
        for (_, decision, _) in &decisions {
            metrics.candidate_decision(match decision {
                ConsolidationDecision::Insert => CandidateDecisionLabel::Insert,
                ConsolidationDecision::Merge => CandidateDecisionLabel::Merge,
                ConsolidationDecision::Reject => CandidateDecisionLabel::Reject,
                ConsolidationDecision::Supersede => CandidateDecisionLabel::Supersede,
            });
        }
        Ok(ConsolidationOutcome {
            consumed: decisions.len(),
            decisions,
        })
    }
}
fn decide(
    scope: &MemoryScope,
    c: &MemoryCandidate,
    approval: Option<&str>,
    seen: &mut HashMap<String, String>,
    current_by_lineage: &HashMap<String, ConsolidatedRecord>,
) -> (ConsolidationDecision, Option<MemoryRecordId>) {
    if matches!(scope, MemoryScope::Global | MemoryScope::Principal(_))
        && matches!(
            c.kind,
            crate::MemoryKind::CoreState | crate::MemoryKind::ArchitectureDecision
        )
        && approval.is_none()
    {
        return (ConsolidationDecision::Reject, None);
    }
    let normalized = normalize(&c.claim);
    if seen.contains_key(&normalized) {
        return (
            ConsolidationDecision::Merge,
            seen.get(&normalized).cloned().map(MemoryRecordId),
        );
    }
    let id = format!(
        "consolidated:{:x}",
        Sha256::digest(
            format!(
                "{}:{}",
                serde_json::to_string(scope).unwrap(),
                c.content_hash
            )
            .as_bytes()
        )
    );
    let supersedes = current_by_lineage
        .get(&lineage_key(&c.kind, &c.source_event_ids))
        .is_some_and(|record| record.content_hash != c.content_hash);
    seen.insert(normalized, id.clone());
    (
        if supersedes {
            ConsolidationDecision::Supersede
        } else {
            ConsolidationDecision::Insert
        },
        Some(MemoryRecordId(id)),
    )
}

fn normalize(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn lineage_key(kind: &crate::MemoryKind, source_event_ids: &[String]) -> String {
    let mut sources = source_event_ids.to_vec();
    sources.sort();
    format!(
        "{}:{}",
        serde_json::to_string(kind).unwrap_or_default(),
        serde_json::to_string(&sources).unwrap_or_default()
    )
}
fn watermark(v: &[(i64, ConsolidationDecision, Option<MemoryRecordId>)]) -> String {
    format!("{:x}", Sha256::digest(serde_json::to_vec(v).unwrap()))
}
#[allow(dead_code)]
fn authority(_: &MemoryCandidate) -> MemoryAuthority {
    MemoryAuthority::VerifiedLocalSemantic
}
