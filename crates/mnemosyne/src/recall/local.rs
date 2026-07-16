use chrono::{DateTime, Utc};
use fabric::ReflectionEntry;

use crate::r#impl::fact_store::FactRow;
use crate::r#impl::recall_memory::MemoryEntry as RecallEntry;
use crate::{
    MemoryAuthority, MemoryMetadata, MemoryProvenance, MemoryScope, MemorySensitivity, RecallItem,
    RecallRequest, TemporalState,
};

pub(crate) fn messages(rows: Vec<RecallEntry>, request: &RecallRequest) -> Vec<RecallItem> {
    rows.into_iter()
        .filter(|row| row.session_id == request.session)
        .map(|row| {
            let metadata = row
                .metadata
                .as_deref()
                .and_then(|json| serde_json::from_str::<MemoryMetadata>(json).ok())
                .unwrap_or_else(|| MemoryMetadata {
                    record_id: format!("mnemosyne:message:{}", row.id),
                    provenance: MemoryProvenance {
                        source: "mnemosyne.recall_memory".into(),
                        source_id: row.id.to_string(),
                        principal: None,
                        source_commit: None,
                    },
                    source_time: Some(row.timestamp),
                    observed_time: row.timestamp,
                    valid_from: Some(row.timestamp),
                    valid_until: None,
                    supersedes: None,
                    superseded_by: None,
                    confidence: 1.0,
                    sensitivity: MemorySensitivity::Internal,
                });
            RecallItem {
                content: row.content,
                metadata,
                temporal_state: TemporalState::Current,
                authority: MemoryAuthority::RawExperience,
                scope: MemoryScope::Session(request.session.clone()),
            }
        })
        .collect()
}

pub(crate) fn facts(
    rows: Vec<FactRow>,
    request: &RecallRequest,
    fallback_now: DateTime<Utc>,
) -> Vec<RecallItem> {
    rows.into_iter()
        .filter_map(|row| {
            let source_time = DateTime::parse_from_rfc3339(&row.created_at)
                .ok()
                .map(|value| value.with_timezone(&Utc));
            let observed_time = DateTime::parse_from_rfc3339(&row.updated_at)
                .ok()
                .map(|value| value.with_timezone(&Utc))
                .or(source_time)
                .unwrap_or(fallback_now);
            let valid_until = source_time.and_then(|created| {
                (row.ttl_days > 0).then(|| created + chrono::Duration::days(row.ttl_days))
            });
            let metadata = MemoryMetadata {
                record_id: format!("mnemosyne:fact:{}", row.fact_id),
                provenance: MemoryProvenance {
                    source: if row.source.is_empty() {
                        "mnemosyne.fact_store".into()
                    } else {
                        row.source
                    },
                    source_id: row.fact_id.to_string(),
                    principal: (!row.subject.is_empty()).then_some(row.subject),
                    source_commit: None,
                },
                source_time,
                observed_time,
                valid_from: source_time,
                valid_until,
                supersedes: None,
                superseded_by: None,
                confidence: row.trust_score.clamp(0.0, 1.0),
                sensitivity: MemorySensitivity::Internal,
            };
            let temporal_state = if row.status == "superseded" {
                TemporalState::Superseded
            } else {
                metadata.temporal_state(request.current_at)
            };
            let scope = metadata
                .provenance
                .principal
                .clone()
                .map(MemoryScope::Principal)
                .unwrap_or_else(|| MemoryScope::Session(request.session.clone()));
            (request.include_historical
                || !matches!(
                    temporal_state,
                    TemporalState::Superseded | TemporalState::Expired
                ))
            .then_some(RecallItem {
                content: row.content,
                metadata,
                temporal_state,
                authority: MemoryAuthority::VerifiedLocalSemantic,
                scope,
            })
        })
        .collect()
}

pub(crate) fn reflections(rows: Vec<ReflectionEntry>, request: &RecallRequest) -> Vec<RecallItem> {
    let terms = request
        .query
        .split_whitespace()
        .map(str::to_lowercase)
        .collect::<Vec<_>>();
    rows.into_iter()
        .filter(|row| {
            let searchable =
                format!("{} {}", row.task_summary, row.learned.join(" ")).to_lowercase();
            terms.iter().all(|term| searchable.contains(term))
        })
        .map(|row| RecallItem {
            content: row.task_summary,
            metadata: MemoryMetadata {
                record_id: row.id.clone(),
                provenance: MemoryProvenance {
                    source: "mnemosyne.episodic".into(),
                    source_id: row.id,
                    principal: None,
                    source_commit: None,
                },
                source_time: Some(row.timestamp),
                observed_time: row.timestamp,
                valid_from: Some(row.timestamp),
                valid_until: None,
                supersedes: None,
                superseded_by: None,
                confidence: row.confidence.clamp(0.0, 1.0),
                sensitivity: MemorySensitivity::Internal,
            },
            temporal_state: TemporalState::Current,
            authority: MemoryAuthority::LocalEpisode,
            scope: MemoryScope::Session(request.session.clone()),
        })
        .collect()
}

pub(crate) fn core(
    blocks: impl IntoIterator<Item = (String, String)>,
    request: &RecallRequest,
    now: DateTime<Utc>,
) -> Vec<RecallItem> {
    let terms = request
        .query
        .split_whitespace()
        .map(str::to_lowercase)
        .collect::<Vec<_>>();
    blocks
        .into_iter()
        .filter(|(_, value)| {
            let value = value.to_lowercase();
            !value.trim().is_empty() && terms.iter().all(|term| value.contains(term))
        })
        .map(|(label, content)| RecallItem {
            metadata: MemoryMetadata {
                record_id: format!("mnemosyne:core:{label}"),
                provenance: MemoryProvenance {
                    source: "mnemosyne.core".into(),
                    source_id: label,
                    principal: None,
                    source_commit: None,
                },
                source_time: None,
                observed_time: now,
                valid_from: None,
                valid_until: None,
                supersedes: None,
                superseded_by: None,
                confidence: 1.0,
                sensitivity: MemorySensitivity::Internal,
            },
            content,
            temporal_state: TemporalState::Current,
            authority: MemoryAuthority::ApprovedCore,
            scope: MemoryScope::Global,
        })
        .collect()
}
