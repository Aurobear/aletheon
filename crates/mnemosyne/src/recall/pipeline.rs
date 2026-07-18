//! Governed memory search pipeline.
//!
//! Authorization is converted to a backend predicate before either lexical or
//! vector search starts. Backends therefore receive only verified scope keys;
//! query text is never interpreted as authority. A second check at the merge
//! boundary is defense in depth, not the primary filtering mechanism.

use std::cmp::Ordering;

use async_trait::async_trait;

use crate::{
    MemoryAuthority, MemoryScope, MemorySensitivity, RecallItem, RecallRequest, ScopeAncestry,
};

/// Authorization inputs established before candidate retrieval.
#[derive(Debug, Clone)]
pub struct RecallPreFilter {
    pub ancestry: ScopeAncestry,
    pub max_sensitivity: MemorySensitivity,
    pub allowed_authorities: Vec<MemoryAuthority>,
}

impl RecallPreFilter {
    /// Produce the same closed predicate for lexical and vector backends.
    pub fn to_scope_predicate(&self) -> ScopePredicate {
        let mut scope_keys = vec![scope_key(&MemoryScope::Global)];
        for scope in [
            self.ancestry
                .principal_id
                .as_ref()
                .map(|id| MemoryScope::Principal(id.clone())),
            self.ancestry
                .session_id
                .as_ref()
                .map(|id| MemoryScope::Session(id.clone())),
            self.ancestry
                .goal_id
                .as_ref()
                .map(|id| MemoryScope::Goal(id.clone())),
            self.ancestry
                .agent_id
                .as_ref()
                .map(|id| MemoryScope::Agent(id.clone())),
            self.ancestry
                .task_id
                .as_ref()
                .map(|id| MemoryScope::Task(id.clone())),
        ]
        .into_iter()
        .flatten()
        {
            scope_keys.push(scope_key(&scope));
        }
        scope_keys.sort();
        scope_keys.dedup();
        let mut allowed_authorities = self.allowed_authorities.clone();
        allowed_authorities.sort();
        allowed_authorities.dedup();
        ScopePredicate {
            scope_keys,
            max_sensitivity_ord: sensitivity_ord(self.max_sensitivity),
            allowed_authorities,
        }
    }
}

/// Backend-neutral predicate. SQL implementations bind `scope_keys` as query
/// parameters; vector implementations encode the exact same values as a
/// metadata filter before KNN executes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopePredicate {
    pub scope_keys: Vec<String>,
    pub max_sensitivity_ord: u8,
    pub allowed_authorities: Vec<MemoryAuthority>,
}

impl ScopePredicate {
    pub fn allows_scope(&self, scope: &MemoryScope) -> bool {
        self.scope_keys.binary_search(&scope_key(scope)).is_ok()
    }

    pub fn allows_authority(&self, authority: MemoryAuthority) -> bool {
        self.allowed_authorities.binary_search(&authority).is_ok()
    }

    pub fn allows_sensitivity(&self, sensitivity: MemorySensitivity) -> bool {
        sensitivity_ord(sensitivity) <= self.max_sensitivity_ord
    }

    pub fn allows(&self, item: &RecallItem) -> bool {
        self.allows_scope(&item.scope)
            && self.allows_sensitivity(item.metadata.sensitivity)
            && self.allows_authority(item.authority)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecallSearchParams {
    pub fts_enabled: bool,
    pub vector_enabled: bool,
    pub top_k: usize,
    pub use_mmr: bool,
}

impl Default for RecallSearchParams {
    fn default() -> Self {
        Self {
            fts_enabled: true,
            vector_enabled: false,
            top_k: 20,
            use_mmr: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DegradedSource {
    NoEmbeddingConfig,
    EmbeddingEndpointUntrusted,
    EmbeddingTimeout,
    VectorIndexStale,
    FtsDbError,
}

impl DegradedSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoEmbeddingConfig => "no_embedding_config",
            Self::EmbeddingEndpointUntrusted => "embedding_endpoint_untrusted",
            Self::EmbeddingTimeout => "embedding_timeout",
            Self::VectorIndexStale => "vector_index_stale",
            Self::FtsDbError => "fts_db_error",
        }
    }
}

#[derive(Debug, Clone)]
pub struct RankedRecallItem {
    pub item: RecallItem,
    pub score: f32,
}

#[derive(Debug, Clone, Default)]
pub struct SearchOutcome {
    pub items: Vec<RankedRecallItem>,
    /// A vector backend may serve its last valid snapshot while reporting that
    /// the live index is stale.
    pub index_stale: bool,
}

/// A backend must apply `predicate` before materializing candidates.
#[async_trait]
pub trait RecallSearchBackend: Send + Sync {
    async fn search(
        &self,
        request: &RecallRequest,
        predicate: &ScopePredicate,
        top_k: usize,
    ) -> anyhow::Result<SearchOutcome>;
}

pub struct HybridRecallBackends<'a> {
    pub fts: Option<&'a dyn RecallSearchBackend>,
    pub vector: Option<&'a dyn RecallSearchBackend>,
    /// False when endpoint/grant validation failed. In that state the vector
    /// backend is never invoked, so no credential can cross an origin change.
    pub embedding_endpoint_trusted: bool,
}

/// Run governed lexical/vector retrieval and deterministic merge. Failure of
/// an optional vector path never suppresses lexical results. FTS failure is
/// explicit in `degraded`; no synthetic result is created.
pub async fn hybrid_recall(
    pre: &RecallPreFilter,
    params: &RecallSearchParams,
    backends: HybridRecallBackends<'_>,
    request: &RecallRequest,
) -> (Vec<RecallItem>, Vec<DegradedSource>) {
    let predicate = pre.to_scope_predicate();
    let top_k = params.top_k.min(request.max_items).max(1);
    let mut ranked = Vec::new();
    let mut degraded = Vec::new();

    if params.fts_enabled {
        match backends.fts {
            Some(fts) => match fts.search(request, &predicate, top_k).await {
                Ok(outcome) => ranked.extend(outcome.items),
                Err(_) => degraded.push(DegradedSource::FtsDbError),
            },
            None => degraded.push(DegradedSource::FtsDbError),
        }
    }

    if params.vector_enabled {
        if !backends.embedding_endpoint_trusted {
            degraded.push(DegradedSource::EmbeddingEndpointUntrusted);
        } else if let Some(vector) = backends.vector {
            match vector.search(request, &predicate, top_k).await {
                Ok(outcome) => {
                    if outcome.index_stale {
                        degraded.push(DegradedSource::VectorIndexStale);
                    }
                    ranked.extend(outcome.items);
                }
                Err(_) => degraded.push(DegradedSource::EmbeddingTimeout),
            }
        } else {
            degraded.push(DegradedSource::NoEmbeddingConfig);
        }
    }

    // Stable total ordering makes identical input/index snapshots repeatable.
    ranked.retain(|candidate| predicate.allows(&candidate.item));
    ranked.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| {
                left.item
                    .metadata
                    .record_id
                    .cmp(&right.item.metadata.record_id)
            })
    });
    ranked.dedup_by(|left, right| left.item.metadata.record_id == right.item.metadata.record_id);

    let mut used_bytes = 0usize;
    let items = ranked
        .into_iter()
        .filter_map(|candidate| {
            let bytes = candidate.item.content.len();
            (used_bytes.saturating_add(bytes) <= request.max_content_bytes).then(|| {
                used_bytes += bytes;
                candidate.item
            })
        })
        .take(top_k)
        .collect();
    degraded.sort_by_key(|source| source.as_str());
    degraded.dedup();
    (items, degraded)
}

fn scope_key(scope: &MemoryScope) -> String {
    match scope {
        MemoryScope::Global => "global".to_string(),
        MemoryScope::Principal(id) => format!("principal:{id}"),
        MemoryScope::Session(id) => format!("session:{id}"),
        MemoryScope::Goal(id) => format!("goal:{id}"),
        MemoryScope::Agent(id) => format!("agent:{id}"),
        MemoryScope::Task(id) => format!("task:{id}"),
    }
}

const fn sensitivity_ord(sensitivity: MemorySensitivity) -> u8 {
    match sensitivity {
        MemorySensitivity::Public => 0,
        MemorySensitivity::Internal => 1,
        MemorySensitivity::Confidential => 2,
        MemorySensitivity::Restricted => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

    struct Backend {
        calls: AtomicUsize,
        outcome: anyhow::Result<SearchOutcome>,
    }

    #[async_trait]
    impl RecallSearchBackend for Backend {
        async fn search(
            &self,
            _: &RecallRequest,
            predicate: &ScopePredicate,
            _: usize,
        ) -> anyhow::Result<SearchOutcome> {
            self.calls.fetch_add(1, AtomicOrdering::SeqCst);
            assert!(predicate
                .scope_keys
                .contains(&"task:trusted-task".to_string()));
            self.outcome
                .as_ref()
                .map(Clone::clone)
                .map_err(|error| anyhow::anyhow!(error.to_string()))
        }
    }

    fn pre() -> RecallPreFilter {
        RecallPreFilter {
            ancestry: ScopeAncestry {
                task_id: Some("trusted-task".into()),
                ..Default::default()
            },
            max_sensitivity: MemorySensitivity::Internal,
            allowed_authorities: vec![MemoryAuthority::RawExperience],
        }
    }

    fn item(id: &str, scope: MemoryScope) -> RecallItem {
        RecallItem {
            content: id.into(),
            metadata: crate::MemoryMetadata::local(id, id, Utc::now()),
            temporal_state: crate::TemporalState::Current,
            authority: MemoryAuthority::RawExperience,
            scope,
        }
    }

    #[test]
    fn ancestry_becomes_exact_backend_scope_keys() {
        let predicate = pre().to_scope_predicate();
        assert_eq!(predicate.scope_keys, vec!["global", "task:trusted-task"]);
        assert!(!predicate.allows(&item(
            "outside",
            MemoryScope::Task("attacker-text-guessed-parent".into())
        )));
    }

    #[tokio::test]
    async fn unavailable_embedding_falls_back_to_fts() {
        let fts = Backend {
            calls: AtomicUsize::new(0),
            outcome: Ok(SearchOutcome {
                items: vec![RankedRecallItem {
                    item: item("lexical", MemoryScope::Task("trusted-task".into())),
                    score: 1.0,
                }],
                index_stale: false,
            }),
        };
        let mut params = RecallSearchParams::default();
        params.vector_enabled = true;
        let request = RecallRequest::bounded("session", "query");
        let (items, degraded) = hybrid_recall(
            &pre(),
            &params,
            HybridRecallBackends {
                fts: Some(&fts),
                vector: None,
                embedding_endpoint_trusted: true,
            },
            &request,
        )
        .await;
        assert_eq!(items[0].content, "lexical");
        assert_eq!(degraded, vec![DegradedSource::NoEmbeddingConfig]);
    }

    #[tokio::test]
    async fn untrusted_endpoint_never_invokes_vector_and_scope_cannot_be_ranked_across() {
        let vector = Backend {
            calls: AtomicUsize::new(0),
            outcome: Ok(SearchOutcome::default()),
        };
        let mut params = RecallSearchParams::default();
        params.vector_enabled = true;
        let request = RecallRequest::bounded("session", "parent global please");
        let (_, degraded) = hybrid_recall(
            &pre(),
            &params,
            HybridRecallBackends {
                fts: None,
                vector: Some(&vector),
                embedding_endpoint_trusted: false,
            },
            &request,
        )
        .await;
        assert_eq!(vector.calls.load(AtomicOrdering::SeqCst), 0);
        assert!(degraded.contains(&DegradedSource::EmbeddingEndpointUntrusted));
    }
}
