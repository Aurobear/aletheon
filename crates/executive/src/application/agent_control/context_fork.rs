use fabric::{AgentContextFork, AgentControlError, AgentControlErrorKind, ContentId};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const MAX_CONTEXT_GOAL_BYTES: usize = 8 * 1024;
pub const MAX_CONTEXT_CONSTRAINTS: usize = 32;
pub const MAX_CONTEXT_ITEMS: usize = 64;
pub const MAX_CONTEXT_ITEM_BYTES: usize = 8 * 1024;
pub const MAX_CONTEXT_TOTAL_BYTES: usize = 64 * 1024;
pub const MAX_CONTEXT_BROADCAST_REFS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum AgentContextItemKind {
    Selected,
    RecentTurns,
    Memory,
    Evidence,
    ArtifactReference,
    HiddenReasoning,
    RawToolOutput,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentContextItem {
    pub kind: AgentContextItemKind,
    pub label: String,
    pub content: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentContextProjection {
    pub goal: Option<String>,
    pub constraints: Vec<String>,
    pub items: Vec<AgentContextItem>,
    pub broadcast_refs: Vec<ContentId>,
    pub omitted_count: usize,
}

impl AgentContextProjection {
    pub fn from_fork(fork: &AgentContextFork) -> Result<Self, AgentControlError> {
        AgentContextProjectionBuilder::new().fork(fork)?.build()
    }

    pub fn total_bytes(&self) -> usize {
        self.goal.as_ref().map_or(0, String::len)
            + self.constraints.iter().map(String::len).sum::<usize>()
            + self
                .items
                .iter()
                .map(|item| item.label.len() + item.content.len())
                .sum::<usize>()
    }
}

#[derive(Debug, Default)]
pub struct AgentContextProjectionBuilder {
    projection: AgentContextProjection,
    next_order: usize,
}

impl AgentContextProjectionBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn goal(mut self, goal: impl Into<String>) -> Result<Self, AgentControlError> {
        let goal = goal.into();
        reject_unsafe(&goal)?;
        self.projection.goal = Some(truncate_utf8(&goal, MAX_CONTEXT_GOAL_BYTES));
        Ok(self)
    }

    pub fn constraint(mut self, constraint: impl Into<String>) -> Result<Self, AgentControlError> {
        let constraint = constraint.into();
        reject_unsafe(&constraint)?;
        if self.projection.constraints.len() >= MAX_CONTEXT_CONSTRAINTS {
            self.projection.omitted_count += 1;
            return Ok(self);
        }
        self.projection
            .constraints
            .push(truncate_utf8(&constraint, MAX_CONTEXT_ITEM_BYTES));
        Ok(self)
    }

    pub fn item(
        mut self,
        kind: AgentContextItemKind,
        label: impl Into<String>,
        content: impl Into<String>,
    ) -> Result<Self, AgentControlError> {
        if matches!(
            kind,
            AgentContextItemKind::HiddenReasoning | AgentContextItemKind::RawToolOutput
        ) {
            return Err(context_error(
                "hidden reasoning and raw tool output cannot enter child context",
            ));
        }
        let content = content.into();
        reject_unsafe(&content)?;
        if kind == AgentContextItemKind::ArtifactReference && !valid_artifact_reference(&content) {
            return Err(context_error(
                "artifact context must be a content-addressed sha256 reference",
            ));
        }
        if self.projection.items.len() >= MAX_CONTEXT_ITEMS {
            self.projection.omitted_count += 1;
            return Ok(self);
        }
        let label = label.into();
        let label = if label.trim().is_empty() {
            self.next_order += 1;
            format!("context_item_{:04}", self.next_order)
        } else {
            label
        };
        self.projection.items.push(AgentContextItem {
            kind,
            label: truncate_utf8(&label, 512),
            content: truncate_utf8(&content, MAX_CONTEXT_ITEM_BYTES),
        });
        Ok(self)
    }

    pub fn broadcast_ref(mut self, content: ContentId) -> Self {
        if self.projection.broadcast_refs.len() >= MAX_CONTEXT_BROADCAST_REFS {
            self.projection.omitted_count += 1;
        } else if !self.projection.broadcast_refs.contains(&content) {
            self.projection.broadcast_refs.push(content);
        }
        self
    }

    pub fn fork(mut self, fork: &AgentContextFork) -> Result<Self, AgentControlError> {
        fork.validate()?;
        match fork {
            AgentContextFork::None => {}
            AgentContextFork::LastTurns { count } => {
                self = self.item(
                    AgentContextItemKind::RecentTurns,
                    "requested_recent_turn_count",
                    count.to_string(),
                )?;
            }
            AgentContextFork::SelectedProjection { items } => {
                for (index, content) in items.iter().enumerate() {
                    self = self.item(
                        AgentContextItemKind::Selected,
                        format!("selected_context_{:04}", index + 1),
                        content,
                    )?;
                }
            }
        }
        Ok(self)
    }

    pub fn build(mut self) -> Result<AgentContextProjection, AgentControlError> {
        self.projection.constraints.sort();
        self.projection.constraints.dedup();
        self.projection
            .items
            .sort_by(|left, right| (left.kind, &left.label).cmp(&(right.kind, &right.label)));
        self.projection.broadcast_refs.sort();
        self.projection.broadcast_refs.dedup();

        while self.projection.total_bytes() > MAX_CONTEXT_TOTAL_BYTES {
            let Some(last) = self.projection.items.pop() else {
                if self.projection.constraints.pop().is_none() {
                    self.projection.goal = None;
                }
                self.projection.omitted_count += 1;
                if self.projection.total_bytes() <= MAX_CONTEXT_TOTAL_BYTES {
                    break;
                }
                continue;
            };
            let _ = last;
            self.projection.omitted_count += 1;
        }
        Ok(self.projection)
    }
}

fn truncate_utf8(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    let mut boundary = max_bytes;
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value[..boundary].to_string()
}

fn reject_unsafe(value: &str) -> Result<(), AgentControlError> {
    let normalized = value.trim_start().to_ascii_lowercase();
    if normalized.starts_with("hidden_reasoning:")
        || normalized.starts_with("raw_tool_output:")
        || normalized.starts_with("<thinking")
        || normalized.starts_with("chain_of_thought:")
    {
        return Err(context_error(
            "hidden reasoning and raw tool output are prohibited",
        ));
    }
    Ok(())
}

fn valid_artifact_reference(value: &str) -> bool {
    let digest = value
        .strip_prefix("sha256:")
        .or_else(|| value.strip_prefix("artifact:sha256:"));
    digest.is_some_and(|digest| {
        digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

pub fn parse_content_id(value: &str) -> Result<ContentId, AgentControlError> {
    Uuid::parse_str(value)
        .map(ContentId)
        .map_err(|_| context_error("invalid broadcast content ID"))
}

fn context_error(message: impl Into<String>) -> AgentControlError {
    AgentControlError {
        kind: AgentControlErrorKind::InvalidRequest,
        message: message.into(),
    }
}
