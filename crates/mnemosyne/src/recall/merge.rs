use std::collections::{HashMap, HashSet};

use crate::{RecallItem, RecallRequest, TemporalState};

use super::rank::{compare, prefer};

pub(crate) fn merge_items(
    sources: impl IntoIterator<Item = Vec<RecallItem>>,
    request: &RecallRequest,
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
            Some(existing) if !prefer(&item, existing) => {}
            _ => {
                by_key.insert(key, item);
            }
        }
    }
    let mut items = by_key
        .into_values()
        .filter(|item| {
            request.include_historical
                || !matches!(
                    item.temporal_state,
                    TemporalState::Superseded | TemporalState::Expired
                )
        })
        .collect::<Vec<_>>();
    items.sort_by(compare);
    let mut bytes = 0usize;
    items
        .into_iter()
        .take(request.max_items)
        .take_while(|item| {
            if bytes.saturating_add(item.content.len()) > request.max_content_bytes {
                false
            } else {
                bytes += item.content.len();
                true
            }
        })
        .collect()
}
