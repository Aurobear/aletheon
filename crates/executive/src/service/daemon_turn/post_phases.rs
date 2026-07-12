//! Post-turn methods on `DaemonTurnOrchestrator`.
//!
//! Covers hooks, auto-memory extraction, reflection recording, evolution,
//! and Agora snapshot persistence — everything that runs after the ReAct
//! loop completes.

use super::orchestrator::DaemonTurnOrchestrator;
use cognit::harness::linear::TurnMetrics;
use fabric::hook::{HookContext, HookPoint};
use fabric::ReflectionTrigger;
use std::collections::HashMap;
use tracing::{info, warn};

impl DaemonTurnOrchestrator {
    pub(crate) async fn run_post_turn_hooks(&self) {
        let (session_id, turn_count) = {
            let (_sid, sm_arc) = self.get_or_create_session(None).await;
            let sm = sm_arc.lock().await;
            (sm.session_id.clone(), sm.turn_count())
        };
        let hr = self.subsystems.corpus.hook_registry.lock().await;
        let ctx = HookContext {
            point: HookPoint::PostTurn,
            session_id,
            turn_count,
            tool_name: None,
            tool_input: None,
            tool_result: None,
            message: None,
            metadata: HashMap::new(),
        };
        hr.execute(&ctx).await;
    }

    pub(crate) async fn extract_auto_memory(&self, message: &str, text: &str) {
        let mut am = self.subsystems.memory.auto_memory.lock().await;
        if let Ok(facts) = am.analyze_and_store(message, text).await {
            if !facts.is_empty() {
                info!(count = facts.len(), "Auto-memory: stored facts");
            }
        }
    }

    pub(crate) async fn record_turn_reflection(&self, task_summary: &str, text: &str, turn: usize) {
        let mut what_worked = Vec::new();
        let mut what_failed = Vec::new();
        let mut learned = Vec::new();
        let resp_len = text.len();
        if resp_len > 500 {
            what_worked.push(format!("Detailed response ({} chars)", resp_len));
        } else if resp_len > 100 {
            what_worked.push(format!("Concise response ({} chars)", resp_len));
        } else {
            what_worked.push(format!("Brief response ({} chars)", resp_len));
        }
        let text_lower = text.to_lowercase();
        for indicator in &[
            "error",
            "failed",
            "unable",
            "cannot",
            "couldn't",
            "sorry, i",
            "i don't know",
        ] {
            if text_lower.contains(indicator) {
                what_failed.push(format!("Response contains '{}'", indicator));
            }
        }
        for indicator in &[
            "i learned",
            "i now understand",
            "i realize",
            "correction:",
            "actually,",
        ] {
            if text_lower.contains(indicator) {
                learned.push(format!("Self-correction detected: '{}'", indicator));
            }
        }
        what_worked.push(format!("Conversation turn #{}", turn));
        let has_failures = !what_failed.is_empty();
        let entry = self.subsystems.reflector.reflect_conversation(
            task_summary,
            ReflectionTrigger::TaskComplete,
            !has_failures,
            what_worked,
            what_failed,
            learned,
        );
        let store_result = {
            let mem = self.subsystems.memory.episodic_memory.lock().await;
            mem.store_reflection(&entry)
        };
        if let Err(e) = store_result {
            warn!(error = %e, "Failed to store chat reflection");
        } else {
            info!(id = %entry.id, task = %task_summary, "Chat reflection stored");
            let mem = self.subsystems.memory.episodic_memory.lock().await;
            if let Ok(count) = mem.reflection_count() {
                if count > 0 && count % 10 == 0 {
                    info!(
                        count = count,
                        "Running ExperienceSummarizer (periodic trigger)"
                    );
                    if let Ok(recent) = mem.recall_reflections(20) {
                        let summarizer = cognit::core::ExperienceSummarizer::new(
                            self.subsystems.ports.clock.clone(),
                        );
                        if let Some(evo_entry) = summarizer.summarize(&recent) {
                            if let Err(e) = mem.store_evolution_log(&evo_entry) {
                                warn!(error = %e, "Failed to store evolution log");
                            } else {
                                info!(id = %evo_entry.id, patterns = evo_entry.patterns_detected.len(), "Evolution log stored");
                            }
                        }
                    }
                }
            }
        }
    }

    pub(crate) async fn run_post_evolution(
        &self,
        task_summary: &str,
        text: &str,
        metrics: &TurnMetrics,
    ) {
        let success = metrics.completed_normally && !text.starts_with("error:");
        if let Err(e) = self
            .subsystems
            .runtime
            .lock()
            .await
            .post_evolution(
                task_summary,
                text,
                success,
                metrics.tool_calls_made,
                metrics.tool_errors,
                metrics.elapsed_ms,
                metrics.iterations,
                &*self.subsystems.pipeline,
            )
            .await
        {
            warn!(error = %e, "post_evolution failed");
        }
    }

    /// Persist Agora commits at turn end.
    ///
    /// Phase 3 stops writing full Agora snapshots into RecallMemory. The
    /// workspace is durable as append-only commits; snapshots/checkpoints are
    /// a separate storage concern.
    pub(crate) async fn commit_agora_snapshot(&self, session: &str, since_version: u64) {
        let Some(agora) = self.subsystems.ports.agora.as_ref() else {
            tracing::warn!(target: "agora", "ServicePorts.agora missing; skipping agora commit persistence");
            return;
        };
        let commits = agora.changes_since(session, since_version).await;
        if commits.is_empty() {
            return;
        }
        let rm = self.subsystems.memory.recall_memory.lock().await;
        for commit in commits {
            match serde_json::to_string(&commit) {
                Ok(serialized) => {
                    if let Err(e) = rm.store(session, "agora_commit", &serialized, None) {
                        tracing::warn!(target: "agora", error = %e, "agora commit persist failed");
                    }
                }
                Err(e) => {
                    tracing::warn!(target: "agora", error = %e, "agora commit serialize failed")
                }
            }
        }
    }
}
