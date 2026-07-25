//! Recovery scan for incomplete turns (M4-T2).
//!
//! When `grok_hardening.compaction_v2` is enabled the daemon boots up and
//! scans persisted session data for turns that have a start boundary
//! (UserMessage) but no terminal item (AssistantMessage or SystemNotice).
//! Those turns are classified as `Interrupted` or `Failed`.
//!
//! Gated behind `grok_hardening.compaction_v2`. The result is returned as
//! a `TurnRecoveryReport` that M5 doctor can surface.

use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
use async_trait::async_trait;
use fabric::{ItemPayload, ItemRecord, SessionAppendStore, SessionId, TurnId};
use serde::Serialize;

use crate::composition::config::GrokHardeningConfig;

/// Classification of an incomplete turn discovered at startup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum RecoveryClassification {
    Interrupted,
    Failed,
}

/// Minimal durable mutation port used by startup recovery. Implementations
/// must persist both the recovered turn terminal state and the owning
/// session's aggregate status before returning success.
#[async_trait]
pub trait TurnRecoveryStore: SessionAppendStore {
    async fn list_session_ids(&self) -> Result<Vec<SessionId>>;

    async fn mark_recovered_turn(
        &self,
        session_id: &SessionId,
        turn_id: TurnId,
        classification: RecoveryClassification,
    ) -> Result<()>;
}

/// A single turn discovered during the recovery scan.
#[derive(Debug, Clone, Serialize)]
pub struct RecoveredTurn {
    pub session_id: String,
    pub turn_id: String,
    pub classification: RecoveryClassification,
    pub item_count: usize,
}

/// Summary of the recovery scan.
#[derive(Debug, Clone, Default, Serialize)]
pub struct TurnRecoveryReport {
    pub sessions_scanned: usize,
    pub turns_scanned: usize,
    pub incomplete_turns: Vec<RecoveredTurn>,
}

#[derive(Debug, Clone, Default, Serialize, serde::Deserialize)]
pub struct TurnRecoveryHealth {
    pub sessions_scanned: usize,
    pub turns_scanned: usize,
    pub incomplete_turns_recovered: usize,
}

impl From<&TurnRecoveryReport> for TurnRecoveryHealth {
    fn from(report: &TurnRecoveryReport) -> Self {
        Self {
            sessions_scanned: report.sessions_scanned,
            turns_scanned: report.turns_scanned,
            incomplete_turns_recovered: report.incomplete_turns.len(),
        }
    }
}

pub fn persist_recovery_health(
    data_dir: &std::path::Path,
    report: &TurnRecoveryReport,
) -> Result<()> {
    let path = data_dir.join("turn-recovery-health.json");
    let temporary = path.with_extension("json.tmp");
    std::fs::write(
        &temporary,
        serde_json::to_vec(&TurnRecoveryHealth::from(report))?,
    )?;
    std::fs::rename(temporary, path)?;
    Ok(())
}

pub fn read_recovery_health(data_dir: &std::path::Path) -> Result<TurnRecoveryHealth> {
    let path = data_dir.join("turn-recovery-health.json");
    if !path.exists() {
        return Ok(TurnRecoveryHealth::default());
    }
    Ok(serde_json::from_slice(&std::fs::read(path)?)?)
}

impl TurnRecoveryReport {
    pub fn is_clean(&self) -> bool {
        self.incomplete_turns.is_empty()
    }
}

/// Scan persisted sessions for turns that have a start boundary but
/// no terminal item. Gate: only runs when `grok_hardening.compaction_v2`
/// is enabled.
pub async fn scan_incomplete_turns(
    store: &dyn TurnRecoveryStore,
    grok_hardening: &GrokHardeningConfig,
) -> Result<TurnRecoveryReport> {
    if !grok_hardening.compaction_v2 {
        return Ok(TurnRecoveryReport::default());
    }

    let session_ids = store.list_session_ids().await?;
    let mut report = TurnRecoveryReport {
        sessions_scanned: session_ids.len(),
        ..TurnRecoveryReport::default()
    };

    for session_id in &session_ids {
        let items = store
            .load_items(session_id, None)
            .await
            .with_context(|| format!("recovery scan failed to load session {}", session_id.0))?;
        if items.is_empty() {
            continue;
        }
        let incomplete = classify_incomplete_turns(&items);
        report.turns_scanned += count_turns(&items);
        for turn in incomplete {
            store
                .mark_recovered_turn(session_id, turn.turn_id, turn.classification.clone())
                .await?;
            report.incomplete_turns.push(RecoveredTurn {
                session_id: session_id.0.clone(),
                turn_id: turn.turn_id.0.to_string(),
                classification: turn.classification,
                item_count: turn.item_count,
            });
        }
    }

    Ok(report)
}

fn count_turns(items: &[ItemRecord]) -> usize {
    items
        .iter()
        .filter_map(|i| {
            if matches!(
                i.payload,
                ItemPayload::UserMessage { .. } | ItemPayload::AssistantMessage { .. }
            ) {
                Some(&i.turn_id)
            } else {
                None
            }
        })
        .collect::<HashSet<_>>()
        .len()
}

struct IncompleteTurn {
    turn_id: TurnId,
    classification: RecoveryClassification,
    item_count: usize,
}

fn classify_incomplete_turns(items: &[ItemRecord]) -> Vec<IncompleteTurn> {
    let mut turns: HashMap<TurnId, Vec<&ItemRecord>> = HashMap::new();
    for item in items {
        turns.entry(item.turn_id).or_default().push(item);
    }

    let mut incomplete = Vec::new();
    for (turn_id, turn_items) in turns {
        let has_user_message = turn_items
            .iter()
            .any(|i| matches!(i.payload, ItemPayload::UserMessage { .. }));
        let has_tool_call = turn_items
            .iter()
            .any(|i| matches!(i.payload, ItemPayload::ToolCall { .. }));
        let has_terminal = has_terminal_item(&turn_items);

        if has_user_message && !has_terminal {
            incomplete.push(IncompleteTurn {
                turn_id,
                classification: if has_tool_call {
                    RecoveryClassification::Failed
                } else {
                    RecoveryClassification::Interrupted
                },
                item_count: turn_items.len(),
            });
        }
    }

    // Stable sort by turn_id for deterministic output.
    incomplete.sort_by_key(|t| t.turn_id.0.to_string());
    incomplete
}

fn has_terminal_item(items: &[&ItemRecord]) -> bool {
    items.iter().any(|i| {
        matches!(
            i.payload,
            ItemPayload::AssistantMessage { .. } | ItemPayload::SystemNotice { .. }
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use fabric::{ItemId, TurnId};
    use uuid::Uuid;

    fn item(turn_id: TurnId, seq: u64, payload: ItemPayload) -> ItemRecord {
        ItemRecord {
            schema_version: fabric::SESSION_SCHEMA_VERSION,
            id: ItemId(Uuid::new_v4()),
            session_id: SessionId("test-session".into()),
            turn_id,
            sequence: seq,
            created_at_ms: 0,
            payload,
        }
    }

    fn user_msg(turn_id: TurnId, seq: u64) -> ItemRecord {
        item(
            turn_id,
            seq,
            ItemPayload::UserMessage {
                content: "hello".into(),
            },
        )
    }

    fn assistant_msg(turn_id: TurnId, seq: u64) -> ItemRecord {
        item(
            turn_id,
            seq,
            ItemPayload::AssistantMessage {
                content: "hi".into(),
            },
        )
    }

    fn tool_call(turn_id: TurnId, seq: u64) -> ItemRecord {
        item(
            turn_id,
            seq,
            ItemPayload::ToolCall {
                call_id: "c1".into(),
                name: "bash".into(),
                input: serde_json::Value::Null,
            },
        )
    }

    fn system_notice(turn_id: TurnId, seq: u64) -> ItemRecord {
        item(
            turn_id,
            seq,
            ItemPayload::SystemNotice {
                content: "failed".into(),
            },
        )
    }

    #[test]
    fn completed_turn_not_incomplete() {
        let t1 = TurnId::new();
        let items = vec![user_msg(t1, 1), assistant_msg(t1, 2)];
        assert!(classify_incomplete_turns(&items).is_empty());
    }

    #[test]
    fn user_only_is_interrupted() {
        let t1 = TurnId::new();
        let items = vec![user_msg(t1, 1)];
        let incomplete = classify_incomplete_turns(&items);
        assert_eq!(incomplete.len(), 1);
        assert_eq!(
            incomplete[0].classification,
            RecoveryClassification::Interrupted
        );
    }

    #[test]
    fn user_with_tool_calls_but_no_terminal_is_failed() {
        let t1 = TurnId::new();
        let items = vec![user_msg(t1, 1), tool_call(t1, 2)];
        let incomplete = classify_incomplete_turns(&items);
        assert_eq!(incomplete.len(), 1);
        assert_eq!(incomplete[0].classification, RecoveryClassification::Failed);
    }

    #[test]
    fn failed_turn_with_system_notice_is_terminal() {
        let t1 = TurnId::new();
        let items = vec![user_msg(t1, 1), tool_call(t1, 2), system_notice(t1, 3)];
        assert!(classify_incomplete_turns(&items).is_empty());
    }

    #[test]
    fn multiple_incomplete_turns() {
        let t1 = TurnId::new();
        let t2 = TurnId::new();
        let items = vec![user_msg(t1, 1), user_msg(t2, 2)];
        let incomplete = classify_incomplete_turns(&items);
        assert_eq!(incomplete.len(), 2);
    }
}
