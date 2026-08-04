//! Typed message-partition identity for prompt construction.
//!
//! Partitions name the stable and dynamic regions of a turn's context so the
//! host can (a) keep the stable prefix byte-stable for provider prefix caches
//! and (b) profile construction cost per region. Partitions are a diagnostic
//! projection — they never change what is sent to the provider. The stable
//! regions never carry per-turn fields (time, UUID, operation id, budget,
//! device state, per-turn recall, attempt numbers).

use fabric::{Message, Role};

/// The five regions of a turn's request context (see
/// docs/plans/deepseek-cache-and-message-optimization-plan.md Phase C4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PromptRegion {
    /// Identity/security rules and fixed protocol (never per-turn fields).
    StablePrefix,
    /// Canonical tool definitions for the selected AgentProfile.
    StableTools,
    /// Prior conversation history.
    Conversation,
    /// Memory, goal, Dasein, plan and skill projections.
    DynamicContext,
    /// The current user input.
    CurrentInput,
}

impl PromptRegion {
    /// Stable snake_case diagnostic label.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::StablePrefix => "stable_prefix",
            Self::StableTools => "stable_tools",
            Self::Conversation => "conversation",
            Self::DynamicContext => "dynamic_context",
            Self::CurrentInput => "current_input",
        }
    }
}

/// One named region of a turn's context with construction profiling.
///
/// `content` is retained for the regions the host renders locally; the
/// provider-visible wire bytes are what `serialized_bytes` measures. Only
/// numeric costs and digests are meant for observability — never secrets or
/// raw user content in logs.
#[derive(Debug, Clone)]
pub struct PromptPartition {
    pub region: PromptRegion,
    pub wire_role: Option<Role>,
    pub content: String,
    pub chars: usize,
    pub serialized_bytes: u64,
    pub construction_ns: u64,
}

/// Per-turn prompt construction profile: cost and byte size per region.
#[derive(Debug, Default, Clone)]
pub struct PromptConstructionProfile {
    pub partitions: Vec<PromptPartition>,
}

impl PromptConstructionProfile {
    /// Serialized wire bytes for one region (0 when the region was not built).
    pub fn region_bytes(&self, region: PromptRegion) -> u64 {
        self.partitions
            .iter()
            .filter(|partition| partition.region == region)
            .map(|partition| partition.serialized_bytes)
            .sum()
    }

    /// Total serialized wire bytes across all rendered regions.
    pub fn total_bytes(&self) -> u64 {
        self.partitions
            .iter()
            .map(|partition| partition.serialized_bytes)
            .sum()
    }

    /// Deterministic marker list of `region:bytes` for diagnostics.
    pub fn summary(&self) -> Vec<(String, u64)> {
        let mut order: Vec<PromptRegion> = vec![
            PromptRegion::StablePrefix,
            PromptRegion::StableTools,
            PromptRegion::Conversation,
            PromptRegion::DynamicContext,
            PromptRegion::CurrentInput,
        ];
        let mut seen = std::collections::BTreeSet::new();
        for partition in &self.partitions {
            seen.insert(partition.region);
        }
        order.retain(|region| seen.contains(region));
        order
            .into_iter()
            .map(|region| (region.as_str().to_string(), self.region_bytes(region)))
            .collect()
    }
}

fn timed_partition(
    region: PromptRegion,
    wire_role: Option<Role>,
    content: String,
    serialized_bytes: u64,
) -> PromptPartition {
    // Construction time is measured by the caller around content rendering;
    // serialization cost is measured here.
    let started = std::time::Instant::now();
    let serialized = if serialized_bytes == 0 {
        serde_json::to_vec(&content).map_or(0, |bytes| bytes.len() as u64)
    } else {
        serialized_bytes
    };
    PromptPartition {
        region,
        wire_role,
        chars: content.chars().count(),
        serialized_bytes: serialized,
        construction_ns: started.elapsed().as_nanos() as u64,
        content,
    }
}

/// Build a typed partition profile from the rendered regions of one turn.
///
/// `stable_tools` is a marker: tool definitions are canonicalized provider-side
/// (`fabric::canonicalize_tool_definitions`) and hashed by
/// `fabric::tool_schema_digest`; the daemon does not render them into the
/// assembled messages, so this region carries no wire bytes here.
pub fn build_partitions(
    system_prefix: &str,
    history: &[Message],
    dynamic_context: &str,
    current_input: &str,
    tool_count: usize,
) -> PromptConstructionProfile {
    let mut partitions = Vec::new();

    let system_message = Message::system(system_prefix);
    let system_bytes = serde_json::to_vec(&system_message).map_or(0, |bytes| bytes.len() as u64);
    partitions.push(timed_partition(
        PromptRegion::StablePrefix,
        Some(Role::System),
        system_prefix.to_owned(),
        system_bytes,
    ));

    let history_started = std::time::Instant::now();
    let history_bytes = history
        .iter()
        .map(|message| serde_json::to_vec(message).map_or(0, |bytes| bytes.len() as u64))
        .sum::<u64>();
    partitions.push(PromptPartition {
        region: PromptRegion::Conversation,
        wire_role: None,
        // Message count is the metric here; history content is retained only
        // as wire bytes, never copied into the profile.
        chars: history.len(),
        serialized_bytes: history_bytes,
        construction_ns: history_started.elapsed().as_nanos() as u64,
        content: String::new(),
    });

    partitions.push(timed_partition(
        PromptRegion::DynamicContext,
        Some(Role::User),
        dynamic_context.to_owned(),
        0,
    ));

    partitions.push(timed_partition(
        PromptRegion::CurrentInput,
        Some(Role::User),
        current_input.to_owned(),
        0,
    ));

    partitions.push(PromptPartition {
        region: PromptRegion::StableTools,
        wire_role: None,
        chars: tool_count,
        serialized_bytes: 0,
        construction_ns: 0,
        content: String::new(),
    });

    PromptConstructionProfile { partitions }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> PromptConstructionProfile {
        let history = vec![Message::user("prior one"), Message::assistant("prior two")];
        build_partitions(
            "stable system prefix\nsecond line",
            &history,
            "<memory-context>\nrecalled\n</memory-context>",
            "current input",
            3,
        )
    }

    #[test]
    fn records_all_five_regions_with_bytes() {
        let profile = sample();
        assert_eq!(profile.partitions.len(), 5);
        // StablePrefix carries the system-message wire bytes.
        assert!(profile.region_bytes(PromptRegion::StablePrefix) > 0);
        // Conversation sums the serialized history messages.
        assert!(profile.region_bytes(PromptRegion::Conversation) > 0);
        // DynamicContext + CurrentInput are rendered locally (bytes measured).
        assert!(profile.region_bytes(PromptRegion::DynamicContext) > 0);
        assert!(profile.region_bytes(PromptRegion::CurrentInput) > 0);
        assert_eq!(profile.region_bytes(PromptRegion::StableTools), 0);
        // Total is the sum of the parts.
        assert_eq!(
            profile.total_bytes(),
            profile
                .partitions
                .iter()
                .map(|p| p.serialized_bytes)
                .sum::<u64>()
        );
    }

    #[test]
    fn stable_prefix_is_byte_identical_across_turns() {
        let a = sample();
        let b = sample();
        let prefix_a = a
            .partitions
            .iter()
            .find(|p| p.region == PromptRegion::StablePrefix)
            .unwrap();
        let prefix_b = b
            .partitions
            .iter()
            .find(|p| p.region == PromptRegion::StablePrefix)
            .unwrap();
        assert_eq!(prefix_a.serialized_bytes, prefix_b.serialized_bytes);
        assert_eq!(prefix_a.content, prefix_b.content);
    }

    #[test]
    fn summary_is_deterministic_and_ordered() {
        let profile = sample();
        let summary = profile.summary();
        // Canonical region order, not insertion order.
        assert_eq!(
            summary
                .iter()
                .map(|(region, _)| region.as_str())
                .collect::<Vec<_>>(),
            [
                "stable_prefix",
                "stable_tools",
                "conversation",
                "dynamic_context",
                "current_input",
            ]
        );
        assert!(summary[0].1 > 0);
        assert_eq!(summary[1].1, 0);
    }

    #[test]
    fn conversation_chars_reports_message_count() {
        let profile = sample();
        let conversation = profile
            .partitions
            .iter()
            .find(|p| p.region == PromptRegion::Conversation)
            .unwrap();
        assert_eq!(conversation.chars, 2);
    }
}
