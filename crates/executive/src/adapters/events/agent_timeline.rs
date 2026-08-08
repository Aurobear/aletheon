//! Read-only projection of canonical Agent events for interactive clients.

use async_trait::async_trait;
use fabric::{EventPayload, EventTreeId};

use super::{EventReadFilter, SqliteEventSpine};
use crate::application::admin_service::{AdminServiceError, AgentTimelinePort};

#[async_trait]
impl AgentTimelinePort for SqliteEventSpine {
    async fn read_agent_timeline(
        &self,
        root_agent_id: fabric::AgentId,
        agent_id: fabric::AgentId,
        limit: usize,
    ) -> Result<Vec<fabric::protocol::client::AgentTimelineEntry>, AdminServiceError> {
        let agent_id = agent_id.0.to_string();
        let requested_limit = limit.clamp(1, 1_000);
        let events = self
            .read_tree_tail(
                EventTreeId::for_root_session(&root_agent_id.0.to_string()),
                EventReadFilter {
                    from_sequence: None,
                    through_sequence: None,
                    schema: None,
                    visibility: Some(fabric::EventVisibility::Control),
                    // Agent trees interleave events from every child. Read a
                    // bounded tree page first, then apply the per-Agent limit
                    // below so a busy sibling cannot consume this Agent's
                    // entire visible timeline budget.
                    limit: 10_000,
                },
            )
            .map_err(|error| AdminServiceError::Operation(error.to_string()))?;
        let mut timeline = events
            .into_iter()
            .filter(|event| event.identity.agent_id.as_deref() == Some(agent_id.as_str()))
            .filter_map(|event| {
                let EventPayload::Inline { value } = event.payload else {
                    return None;
                };
                Some(fabric::protocol::client::AgentTimelineEntry {
                    sequence: event.position.sequence.0,
                    kind: value
                        .get("kind")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("runtime")
                        .to_owned(),
                    detail: value
                        .get("detail")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null),
                })
            })
            .collect::<Vec<_>>();
        if timeline.len() > requested_limit {
            timeline.drain(..timeline.len() - requested_limit);
        }
        Ok(timeline)
    }
}
