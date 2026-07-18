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

/// Vector decorator retaining the last non-stale result set. When the live
/// backend reports a stale index without candidates, the bounded last-valid
/// snapshot is served and remains marked stale for observability.
pub struct LastValidSnapshotBackend {
    live: std::sync::Arc<dyn RecallSearchBackend>,
    last_valid: tokio::sync::Mutex<Option<SearchOutcome>>,
}

impl LastValidSnapshotBackend {
    pub fn new(live: std::sync::Arc<dyn RecallSearchBackend>) -> Self {
        Self {
            live,
            last_valid: tokio::sync::Mutex::new(None),
        }
    }
}

#[async_trait]
impl RecallSearchBackend for LastValidSnapshotBackend {
    async fn search(
        &self,
        request: &RecallRequest,
        predicate: &ScopePredicate,
        top_k: usize,
    ) -> anyhow::Result<SearchOutcome> {
        let mut outcome = self.live.search(request, predicate, top_k).await?;
        let mut snapshot = self.last_valid.lock().await;
        if !outcome.index_stale {
            outcome.items.truncate(top_k);
            *snapshot = Some(outcome.clone());
        } else if outcome.items.is_empty() {
            if let Some(last_valid) = snapshot.as_ref() {
                outcome.items = last_valid.items.iter().take(top_k).cloned().collect();
            }
        }
        Ok(outcome)
    }
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
    hybrid_recall_with_metrics(pre, params, backends, request, None).await
}

/// Production variant that records candidates rejected by the governed merge
/// boundary. Backends are required to apply the predicate before retrieval;
/// this counter makes any defense-in-depth rejection observable rather than
/// silently discarding a backend policy violation.
pub(crate) async fn hybrid_recall_with_metrics(
    pre: &RecallPreFilter,
    params: &RecallSearchParams,
    backends: HybridRecallBackends<'_>,
    request: &RecallRequest,
    metrics: Option<&crate::observability::MemoryMetrics>,
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
        if backends.vector.is_none() {
            degraded.push(DegradedSource::NoEmbeddingConfig);
        } else if !backends.embedding_endpoint_trusted {
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
        }
    }

    // Stable total ordering makes identical input/index snapshots repeatable.
    let candidates_before_governance = ranked.len();
    ranked.retain(|candidate| predicate.allows(&candidate.item));
    let excluded = candidates_before_governance.saturating_sub(ranked.len());
    if excluded != 0 {
        if let Some(metrics) = metrics {
            metrics.recall_prefilter_excluded(excluded);
        }
    }
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

    if params.use_mmr {
        ranked = deterministic_mmr(ranked, top_k);
    }

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

/// Deterministic MMR with a fixed lambda of 0.7. Ties use record_id, matching
/// the non-MMR total order; this makes the boolean parameter's behavior fully
/// specified without adding a second assembly-time tuning surface.
fn deterministic_mmr(mut candidates: Vec<RankedRecallItem>, limit: usize) -> Vec<RankedRecallItem> {
    let mut selected: Vec<RankedRecallItem> = Vec::new();
    while !candidates.is_empty() && selected.len() < limit {
        let best = candidates
            .iter()
            .enumerate()
            .max_by(|(_, left), (_, right)| {
                let utility = |candidate: &RankedRecallItem| {
                    let redundancy = selected
                        .iter()
                        .map(|chosen| {
                            lexical_overlap(&candidate.item.content, &chosen.item.content)
                        })
                        .fold(0.0_f32, f32::max);
                    0.7 * candidate.score - 0.3 * redundancy
                };
                utility(left)
                    .partial_cmp(&utility(right))
                    .unwrap_or(Ordering::Equal)
                    .then_with(|| {
                        right
                            .item
                            .metadata
                            .record_id
                            .cmp(&left.item.metadata.record_id)
                    })
            })
            .map(|(index, _)| index)
            .expect("non-empty candidates");
        selected.push(candidates.remove(best));
    }
    selected.extend(candidates);
    selected
}

fn lexical_overlap(left: &str, right: &str) -> f32 {
    let left: std::collections::BTreeSet<_> = left.split_whitespace().collect();
    let right: std::collections::BTreeSet<_> = right.split_whitespace().collect();
    let union = left.union(&right).count();
    if union == 0 {
        0.0
    } else {
        left.intersection(&right).count() as f32 / union as f32
    }
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
    use proptest::prelude::*;
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

    #[tokio::test]
    async fn fts_failure_is_degraded_without_synthetic_success() {
        let fts = Backend {
            calls: AtomicUsize::new(0),
            outcome: Err(anyhow::anyhow!("db offline")),
        };
        let request = RecallRequest::bounded("session", "query");
        let (items, degraded) = hybrid_recall(
            &pre(),
            &RecallSearchParams::default(),
            HybridRecallBackends {
                fts: Some(&fts),
                vector: None,
                embedding_endpoint_trusted: true,
            },
            &request,
        )
        .await;
        assert!(items.is_empty());
        assert_eq!(degraded, vec![DegradedSource::FtsDbError]);
    }

    #[tokio::test]
    async fn vector_candidates_cannot_cross_scope_predicate() {
        let vector = Backend {
            calls: AtomicUsize::new(0),
            outcome: Ok(SearchOutcome {
                items: vec![
                    RankedRecallItem {
                        item: item("allowed", MemoryScope::Task("trusted-task".into())),
                        score: 0.4,
                    },
                    RankedRecallItem {
                        item: item("denied", MemoryScope::Task("other".into())),
                        score: 1.0,
                    },
                ],
                index_stale: false,
            }),
        };
        let mut params = RecallSearchParams::default();
        params.vector_enabled = true;
        let (items, _) = hybrid_recall(
            &pre(),
            &params,
            HybridRecallBackends {
                fts: None,
                vector: Some(&vector),
                embedding_endpoint_trusted: true,
            },
            &RecallRequest::bounded("session", "query"),
        )
        .await;
        assert_eq!(
            items
                .iter()
                .map(|item| item.content.as_str())
                .collect::<Vec<_>>(),
            vec!["allowed"]
        );
    }

    #[tokio::test]
    async fn governed_boundary_counts_backend_candidates_excluded_by_prefilter() {
        let vector = Backend {
            calls: AtomicUsize::new(0),
            outcome: Ok(SearchOutcome {
                items: vec![
                    RankedRecallItem {
                        item: item("allowed", MemoryScope::Task("trusted-task".into())),
                        score: 0.4,
                    },
                    RankedRecallItem {
                        item: item("wrong-scope", MemoryScope::Task("other".into())),
                        score: 1.0,
                    },
                    RankedRecallItem {
                        item: item("wrong-scope-2", MemoryScope::Principal("other".into())),
                        score: 0.9,
                    },
                ],
                index_stale: false,
            }),
        };
        let metrics = crate::observability::MemoryMetrics::default();
        let params = RecallSearchParams {
            fts_enabled: false,
            vector_enabled: true,
            ..RecallSearchParams::default()
        };

        let (items, _) = hybrid_recall_with_metrics(
            &pre(),
            &params,
            HybridRecallBackends {
                fts: None,
                vector: Some(&vector),
                embedding_endpoint_trusted: true,
            },
            &RecallRequest::bounded("session", "query"),
            Some(&metrics),
        )
        .await;

        assert_eq!(items.len(), 1);
        assert_eq!(metrics.snapshot().recall_prefilter_excluded_total, 2);
    }

    #[tokio::test]
    async fn deterministic_merge_and_mmr_repeat() {
        async fn run() -> Vec<String> {
            let vector = Backend {
                calls: AtomicUsize::new(0),
                outcome: Ok(SearchOutcome {
                    items: vec![
                        RankedRecallItem {
                            item: item("b shared words", MemoryScope::Task("trusted-task".into())),
                            score: 0.8,
                        },
                        RankedRecallItem {
                            item: item("a shared words", MemoryScope::Task("trusted-task".into())),
                            score: 0.8,
                        },
                        RankedRecallItem {
                            item: item("c distinct", MemoryScope::Task("trusted-task".into())),
                            score: 0.7,
                        },
                    ],
                    index_stale: false,
                }),
            };
            let mut params = RecallSearchParams::default();
            params.vector_enabled = true;
            params.use_mmr = true;
            hybrid_recall(
                &pre(),
                &params,
                HybridRecallBackends {
                    fts: None,
                    vector: Some(&vector),
                    embedding_endpoint_trusted: true,
                },
                &RecallRequest::bounded("session", "q"),
            )
            .await
            .0
            .into_iter()
            .map(|item| item.content)
            .collect()
        }
        assert_eq!(run().await, run().await);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn merge_and_rank_are_repeatable_for_bounded_index_snapshots(
            snapshot in prop::collection::vec(
                (0_u16..1_000, -1_000_i16..1_000, "[a-z]{1,8}( [a-z]{1,8}){0,3}"),
                0..20,
            ),
            use_mmr in any::<bool>(),
            top_k in 1_usize..20,
        ) {
            fn candidates(snapshot: &[(u16, i16, String)]) -> Vec<RankedRecallItem> {
                snapshot
                    .iter()
                    .map(|(id, score, content)| {
                        let mut recall = item(
                            &format!("record-{id}"),
                            MemoryScope::Task("trusted-task".into()),
                        );
                        recall.content = content.clone();
                        RankedRecallItem {
                            item: recall,
                            score: f32::from(*score) / 100.0,
                        }
                    })
                    .collect()
            }

            let run = || {
                let vector = Backend {
                    calls: AtomicUsize::new(0),
                    outcome: Ok(SearchOutcome {
                        items: candidates(&snapshot),
                        index_stale: false,
                    }),
                };
                let params = RecallSearchParams {
                    fts_enabled: false,
                    vector_enabled: true,
                    top_k,
                    use_mmr,
                };
                let request = RecallRequest::bounded("session", "property query");
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(hybrid_recall(
                        &pre(),
                        &params,
                        HybridRecallBackends {
                            fts: None,
                            vector: Some(&vector),
                            embedding_endpoint_trusted: true,
                        },
                        &request,
                    ))
                    .0
                    .into_iter()
                    .map(|item| (item.metadata.record_id, item.content))
                    .collect::<Vec<_>>()
            };

            prop_assert_eq!(run(), run());
        }
    }

    #[tokio::test]
    async fn stale_vector_serves_last_valid_snapshot_with_signal() {
        let fts = Backend {
            calls: AtomicUsize::new(0),
            outcome: Ok(SearchOutcome {
                items: vec![RankedRecallItem {
                    item: item("lexical", MemoryScope::Task("trusted-task".into())),
                    score: 0.5,
                }],
                index_stale: false,
            }),
        };
        let vector = Backend {
            calls: AtomicUsize::new(0),
            outcome: Ok(SearchOutcome {
                items: vec![RankedRecallItem {
                    item: item("snapshot", MemoryScope::Task("trusted-task".into())),
                    score: 0.9,
                }],
                index_stale: true,
            }),
        };
        let mut params = RecallSearchParams::default();
        params.vector_enabled = true;
        let (items, degraded) = hybrid_recall(
            &pre(),
            &params,
            HybridRecallBackends {
                fts: Some(&fts),
                vector: Some(&vector),
                embedding_endpoint_trusted: true,
            },
            &RecallRequest::bounded("session", "q"),
        )
        .await;
        assert_eq!(items[0].content, "snapshot");
        assert!(items.iter().any(|item| item.content == "lexical"));
        assert_eq!(degraded, vec![DegradedSource::VectorIndexStale]);
    }

    #[tokio::test]
    async fn stale_vector_wrapper_reuses_bounded_last_valid_snapshot() {
        struct Sequence(tokio::sync::Mutex<std::collections::VecDeque<SearchOutcome>>);
        #[async_trait]
        impl RecallSearchBackend for Sequence {
            async fn search(
                &self,
                _: &RecallRequest,
                _: &ScopePredicate,
                _: usize,
            ) -> anyhow::Result<SearchOutcome> {
                Ok(self.0.lock().await.pop_front().unwrap())
            }
        }
        let live = std::sync::Arc::new(Sequence(tokio::sync::Mutex::new(
            std::collections::VecDeque::from([
                SearchOutcome {
                    items: vec![RankedRecallItem {
                        item: item("last-valid", MemoryScope::Task("trusted-task".into())),
                        score: 0.9,
                    }],
                    index_stale: false,
                },
                SearchOutcome {
                    items: Vec::new(),
                    index_stale: true,
                },
            ]),
        )));
        let cached = LastValidSnapshotBackend::new(live);
        let request = RecallRequest::bounded("s", "q");
        let predicate = pre().to_scope_predicate();
        cached.search(&request, &predicate, 1).await.unwrap();
        let stale = cached.search(&request, &predicate, 1).await.unwrap();
        assert!(stale.index_stale);
        assert_eq!(stale.items[0].item.content, "last-valid");
    }
}
