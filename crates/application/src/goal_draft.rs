//! APX-03 Goal Draft + External Stimulus provider-neutral use cases (Agent Kernel V2).
//!
//! `CreateGoalDraft` and `IngestExternalStimulus` are extracted as
//! provider-neutral use cases: they accept a **generic stimulus** shape and
//! never name a concrete channel provider.  Channel OAuth/transport stay in
//! the extension adapter (E2-K6a).  Goal attempt/worker/budget/verification
//! are **not** copied into Application — they stay with the Runtime/extension
//! owner.  Shared `objectives.db` tables are managed per owner bundle at the
//! cutover.

use serde::{Deserialize, Serialize};

/// Provider-neutral external stimulus.  `channel` is an opaque channel label;
/// the concrete channel adapter maps its own transport onto this.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalStimulus {
    /// Opaque channel label.  Application never interprets channel-specific
    /// fields.
    pub channel: String,
    pub content: String,
    pub correlation: Option<String>,
}

/// A Goal Draft created from a stimulus.  Provider-neutral.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalDraft {
    pub title: String,
    pub body: String,
}

/// Create a goal draft from a provider-neutral request.
pub fn create_goal_draft(title: String, body: String) -> GoalDraft {
    GoalDraft { title, body }
}

/// Ingest an external stimulus into a goal draft.  Works with any channel
/// **disabled** — the use case never requires a concrete channel adapter.
pub fn ingest_external_stimulus(stimulus: ExternalStimulus) -> GoalDraft {
    GoalDraft {
        title: format!("goal:{}", stimulus.channel),
        body: stimulus.content,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generic_stimulus_ingests_without_gmail() {
        // No Gmail/Google name anywhere; a generic stimulus works with the
        // extension disabled.
        let draft = ingest_external_stimulus(ExternalStimulus {
            channel: "telegram".into(),
            content: "please review".into(),
            correlation: Some("c1".into()),
        });
        assert_eq!(draft.title, "goal:telegram");
        assert_eq!(draft.body, "please review");
    }

    #[test]
    fn goal_draft_is_provider_neutral() {
        let draft = create_goal_draft("Review the PR".into(), "body".into());
        assert_eq!(draft.title, "Review the PR");
    }
}
