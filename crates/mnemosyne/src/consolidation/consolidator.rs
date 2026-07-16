use super::{ConsolidationRepository, MemoryCandidate};
use crate::{MemoryAuthority, MemoryRecordId, MemoryScope};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

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
        let mut seen = std::collections::HashMap::<String, String>::new();
        let mut decisions = Vec::new();
        for (id, candidate) in candidates {
            let (decision, record) = decide(scope, &candidate, approval_evidence, &mut seen);
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
    seen: &mut std::collections::HashMap<String, String>,
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
    let normalized = c
        .claim
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
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
    seen.insert(normalized, id.clone());
    (ConsolidationDecision::Insert, Some(MemoryRecordId(id)))
}
fn watermark(v: &[(i64, ConsolidationDecision, Option<MemoryRecordId>)]) -> String {
    format!("{:x}", Sha256::digest(serde_json::to_vec(v).unwrap()))
}
#[allow(dead_code)]
fn authority(_: &MemoryCandidate) -> MemoryAuthority {
    MemoryAuthority::VerifiedLocalSemantic
}
