use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::MemoryScope;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MemoryRecordId(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    Message,
    ToolOutcome,
    GoalOutcome,
    Reflection,
    Episodic,
    SemanticFact,
    Procedure,
    CoreState,
    ArchitectureDecision,
    ExternalReference,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryStatus {
    Candidate,
    Current,
    Superseded,
    Expired,
    Rejected,
    Tombstoned,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryAuthority {
    ApprovedCore,
    VerifiedLocalSemantic,
    LocalEpisode,
    AletheonExternal,
    ExternalReference,
    #[default]
    RawExperience,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemorySensitivity {
    Public,
    Internal,
    Confidential,
    Restricted,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryProvenance {
    pub source: String,
    pub source_id: String,
    pub principal: Option<String>,
    pub source_commit: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryMetadata {
    pub record_id: String,
    pub provenance: MemoryProvenance,
    pub source_time: Option<DateTime<Utc>>,
    pub observed_time: DateTime<Utc>,
    pub valid_from: Option<DateTime<Utc>>,
    pub valid_until: Option<DateTime<Utc>>,
    pub supersedes: Option<String>,
    pub superseded_by: Option<String>,
    pub confidence: f64,
    pub sensitivity: MemorySensitivity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TemporalState {
    Current,
    Superseded,
    Expired,
    Unknown,
}

impl MemoryMetadata {
    pub fn local(
        record_id: impl Into<String>,
        source_id: impl Into<String>,
        observed_time: DateTime<Utc>,
    ) -> Self {
        Self {
            record_id: record_id.into(),
            provenance: MemoryProvenance {
                source: "aletheon".into(),
                source_id: source_id.into(),
                principal: None,
                source_commit: None,
            },
            source_time: Some(observed_time),
            observed_time,
            valid_from: Some(observed_time),
            valid_until: None,
            supersedes: None,
            superseded_by: None,
            confidence: 1.0,
            sensitivity: MemorySensitivity::Internal,
        }
    }

    pub fn temporal_state(&self, current_at: Option<DateTime<Utc>>) -> TemporalState {
        if self.superseded_by.is_some() {
            return TemporalState::Superseded;
        }
        let Some(now) = current_at else {
            return TemporalState::Unknown;
        };
        if self.valid_until.is_some_and(|until| until <= now) {
            return TemporalState::Expired;
        }
        if self.valid_from.is_some_and(|from| from > now) {
            return TemporalState::Unknown;
        }
        TemporalState::Current
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.record_id.trim().is_empty(),
            "memory record ID is required"
        );
        anyhow::ensure!(
            !self.provenance.source.trim().is_empty(),
            "memory source is required"
        );
        anyhow::ensure!(
            !self.provenance.source_id.trim().is_empty(),
            "memory source ID is required"
        );
        anyhow::ensure!(
            self.confidence.is_finite() && (0.0..=1.0).contains(&self.confidence),
            "memory confidence must be between 0 and 1"
        );
        if let (Some(from), Some(until)) = (self.valid_from, self.valid_until) {
            anyhow::ensure!(from < until, "memory valid-from must precede valid-until");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryRecord {
    pub id: MemoryRecordId,
    pub kind: MemoryKind,
    pub scope: MemoryScope,
    pub content: String,
    pub metadata: MemoryMetadata,
    pub status: MemoryStatus,
    pub authority: MemoryAuthority,
    pub source_event_ids: Vec<String>,
    pub tags: Vec<String>,
}

impl MemoryRecord {
    pub const MAX_CONTENT_BYTES: usize = 256 * 1024;
    pub const MAX_SOURCE_EVENTS: usize = 256;
    pub const MAX_TAGS: usize = 128;

    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(!self.id.0.trim().is_empty(), "memory record ID is required");
        anyhow::ensure!(
            self.id.0 == self.metadata.record_id,
            "record and metadata IDs differ"
        );
        anyhow::ensure!(
            !self.content.trim().is_empty(),
            "memory record content is required"
        );
        anyhow::ensure!(
            self.content.len() <= Self::MAX_CONTENT_BYTES,
            "memory record content exceeds byte limit"
        );
        anyhow::ensure!(
            self.source_event_ids.len() <= Self::MAX_SOURCE_EVENTS,
            "memory source event count exceeds limit"
        );
        anyhow::ensure!(
            self.tags.len() <= Self::MAX_TAGS,
            "memory tag count exceeds limit"
        );
        anyhow::ensure!(
            self.source_event_ids.iter().all(|id| !id.trim().is_empty()),
            "memory source event ID is required"
        );
        anyhow::ensure!(
            self.tags.iter().all(|tag| !tag.trim().is_empty()),
            "memory tag is required"
        );
        self.scope.validate()?;
        self.metadata.validate()
    }
}
