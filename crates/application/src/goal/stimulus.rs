//! Provider-neutral external stimuli that may wake a waiting Goal.

use crate::ExternalIdentityId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoalExternalEvent {
    pub account_id: ExternalIdentityId,
    pub event_id: String,
    pub object_id: String,
    pub source_timestamp_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalEventWaitCondition {
    pub account_id: ExternalIdentityId,
    pub event_id: Option<String>,
    pub object_id: Option<String>,
    pub source_after_ms: Option<i64>,
    pub source_before_ms: Option<i64>,
}

impl ExternalEventWaitCondition {
    pub fn key(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    pub fn matches(&self, event: &GoalExternalEvent) -> bool {
        self.account_id == event.account_id
            && self
                .event_id
                .as_deref()
                .is_none_or(|id| id == event.event_id)
            && self
                .object_id
                .as_deref()
                .is_none_or(|id| id == event.object_id)
            && self
                .source_after_ms
                .is_none_or(|after| event.source_timestamp_ms >= after)
            && self
                .source_before_ms
                .is_none_or(|before| event.source_timestamp_ms <= before)
    }
}
