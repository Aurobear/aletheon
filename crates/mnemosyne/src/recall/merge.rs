use std::collections::{HashMap, HashSet};

use crate::{MemoryMetrics, RecallItem, RecallOmittedReason, RecallRequest, TemporalState};

use super::rank::{compare, prefer};

pub(crate) fn merge_items(
    sources: impl IntoIterator<Item = Vec<RecallItem>>,
    request: &RecallRequest,
    metrics: Option<&MemoryMetrics>,
) -> Vec<RecallItem> {
    let all = sources.into_iter().flatten().collect::<Vec<_>>();
    let superseded = all
        .iter()
        .filter_map(|item| item.metadata.supersedes.clone())
        .collect::<HashSet<_>>();
    let mut by_key: HashMap<(String, String), RecallItem> = HashMap::new();
    for mut item in all {
        if superseded.contains(&item.metadata.record_id) {
            item.temporal_state = TemporalState::Superseded;
        }
        let key = (
            item.metadata.provenance.source.clone(),
            if request.include_historical {
                format!(
                    "{}#{}",
                    item.metadata.provenance.source_id, item.metadata.record_id
                )
            } else {
                item.metadata.provenance.source_id.clone()
            },
        );
        match by_key.get(&key) {
            Some(existing) if !prefer(&item, existing) => {
                omitted(metrics, RecallOmittedReason::Duplicate, 1);
            }
            _ => {
                if by_key.contains_key(&key) {
                    omitted(metrics, RecallOmittedReason::Duplicate, 1);
                }
                by_key.insert(key, item);
            }
        }
    }
    let (mut items, historical): (Vec<_>, Vec<_>) = by_key.into_values().partition(|item| {
        request.include_historical
            || !matches!(
                item.temporal_state,
                TemporalState::Superseded | TemporalState::Expired
            )
    });
    omitted(metrics, RecallOmittedReason::Historical, historical.len());
    items.sort_by(compare);
    if items.len() > request.max_items {
        omitted(
            metrics,
            RecallOmittedReason::ItemLimit,
            items.len() - request.max_items,
        );
        items.truncate(request.max_items);
    }
    let mut bytes = 0usize;
    let first_overflow = items.iter().position(|item| {
        if bytes.saturating_add(item.content.len()) > request.max_content_bytes {
            true
        } else {
            bytes += item.content.len();
            false
        }
    });
    if let Some(index) = first_overflow {
        omitted(metrics, RecallOmittedReason::ByteLimit, items.len() - index);
        items.truncate(index);
    }
    items
}

fn omitted(metrics: Option<&MemoryMetrics>, reason: RecallOmittedReason, count: usize) {
    if count > 0 {
        if let Some(metrics) = metrics {
            metrics.recall_omitted(reason, count);
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};

    use super::*;
    use crate::{
        MemoryAuthority, MemoryMetadata, MemoryProvenance, MemoryScope, MemorySensitivity,
    };

    fn item(id: &str, source_id: &str, content: &str) -> RecallItem {
        RecallItem {
            content: content.into(),
            metadata: MemoryMetadata {
                record_id: id.into(),
                provenance: MemoryProvenance {
                    source: "test".into(),
                    source_id: source_id.into(),
                    principal: None,
                    source_commit: None,
                },
                source_time: None,
                observed_time: DateTime::<Utc>::UNIX_EPOCH,
                valid_from: None,
                valid_until: None,
                supersedes: None,
                superseded_by: None,
                confidence: 1.0,
                sensitivity: MemorySensitivity::Internal,
            },
            temporal_state: TemporalState::Current,
            authority: MemoryAuthority::RawExperience,
            scope: MemoryScope::Global,
        }
    }

    #[test]
    fn merge_reports_duplicate_item_and_byte_omissions() {
        let metrics = MemoryMetrics::default();
        let mut request = RecallRequest::bounded("session", "query");

        let duplicate = merge_items(
            [vec![item("a", "same", "one"), item("b", "same", "two")]],
            &request,
            Some(&metrics),
        );
        assert_eq!(duplicate.len(), 1);

        request.max_items = 1;
        let limited = merge_items(
            [vec![item("c", "c", "one"), item("d", "d", "two")]],
            &request,
            Some(&metrics),
        );
        assert_eq!(limited.len(), 1);

        request.max_items = 10;
        request.max_content_bytes = 3;
        let byte_limited = merge_items(
            [vec![item("e", "e", "one"), item("f", "f", "two")]],
            &request,
            Some(&metrics),
        );
        assert_eq!(byte_limited.len(), 1);

        let snapshot = metrics.snapshot();
        assert_eq!(
            snapshot.memory_recall_omitted_total[&RecallOmittedReason::Duplicate],
            1
        );
        assert_eq!(
            snapshot.memory_recall_omitted_total[&RecallOmittedReason::ItemLimit],
            1
        );
        assert_eq!(
            snapshot.memory_recall_omitted_total[&RecallOmittedReason::ByteLimit],
            1
        );
    }
}
