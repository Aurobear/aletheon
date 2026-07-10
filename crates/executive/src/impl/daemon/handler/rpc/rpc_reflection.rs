//! Reflection and self-awareness RPC handlers.
//!
//! Methods: reflect, reflect_now, genome, evolution.

use super::RequestHandler;

use serde_json::json;
use tracing::{info, warn};

use fabric::ReflectionTrigger;

impl RequestHandler {
    pub(super) async fn handle_reflect(
        &self,
        id: &serde_json::Value,
        _request: &serde_json::Value,
    ) -> serde_json::Value {
        let reflections = self
            .subsystems
            .episodic_memory
            .lock()
            .await
            .recall_reflections(10);
        match reflections {
            Ok(entries) => {
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "reflections": entries }
                })
            }
            Err(e) => {
                warn!(error = %e, "Failed to recall reflections");
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32001, "message": format!("Reflection recall error: {}", e) }
                })
            }
        }
    }

    pub(super) async fn handle_reflect_now(
        &self,
        id: &serde_json::Value,
        _request: &serde_json::Value,
    ) -> serde_json::Value {
        // Run an immediate reflection on the current session state
        let (turn, session_id, iteration) = {
            let session_id = self
                .subsystems
                .runtime
                .lock()
                .await
                .config()
                .session_id
                .clone();
            let iteration = self.subsystems.runtime.lock().await.iteration();
            let turn = {
                let (_sid, sm_arc) = self.get_or_create_session(None).await;
                let tc = sm_arc.lock().await.turn_count();
                tc
            };
            (turn, session_id, iteration)
        };

        let task_summary = format!(
            "Session {} after {} turns (iteration {})",
            session_id, turn, iteration
        );

        let mut what_worked = Vec::new();
        let mut what_failed = Vec::new();
        let mut learned = Vec::new();

        what_worked.push(format!("Session is active with {} turns", turn));
        what_worked.push(format!("Runtime iteration count: {}", iteration));

        if turn == 0 {
            what_failed.push("No chat turns recorded yet".to_string());
        }

        // Check if there are recent reflections to draw from
        match self
            .subsystems
            .episodic_memory
            .lock()
            .await
            .recall_reflections(5)
        {
            Ok(recent) if !recent.is_empty() => {
                learned.push(format!("Reviewed {} recent reflections", recent.len()));
                // Aggregate failure patterns
                let failure_count: usize = recent.iter().map(|r| r.what_failed.len()).sum();
                if failure_count > 0 {
                    what_failed.push(format!(
                        "{} failure items across recent reflections",
                        failure_count
                    ));
                }
            }
            Ok(_) => {
                learned.push("No prior reflections available for context".to_string());
            }
            Err(e) => {
                what_failed.push(format!("Could not recall reflections: {}", e));
            }
        }

        let has_failures = !what_failed.is_empty() || turn == 0;
        let entry = self.subsystems.reflector.reflect_conversation(
            &task_summary,
            ReflectionTrigger::Manual,
            !has_failures,
            what_worked,
            what_failed,
            learned,
        );
        if let Err(e) = self
            .subsystems
            .episodic_memory
            .lock()
            .await
            .store_reflection(&entry)
        {
            warn!(error = %e, "Failed to store manual reflection");
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32003, "message": format!("Reflect now error: {}", e) }
            })
        } else {
            info!(id = %entry.id, "Manual reflection stored via reflect_now");
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "reflection": {
                        "id": entry.id,
                        "timestamp": entry.timestamp.to_rfc3339(),
                        "task_summary": entry.task_summary,
                        "outcome": entry.outcome.to_string(),
                        "what_worked": entry.what_worked,
                        "what_failed": entry.what_failed,
                        "learned": entry.learned,
                        "confidence": entry.confidence,
                        "turn_count": turn,
                    }
                }
            })
        }
    }

    pub(super) async fn handle_genome(
        &self,
        id: &serde_json::Value,
        _request: &serde_json::Value,
    ) -> serde_json::Value {
        // Read the genome dynamically from SelfField using SelfReader.
        let self_field = self.subsystems.self_field.lock().await;
        let reader = metacog::r#impl::meta_runtime::self_reader::SelfReader::new();
        match reader.read_genome(&*self_field).await {
            Ok(genome) => match serde_yaml::to_string(&genome) {
                Ok(yaml) => {
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": { "genome": yaml }
                    })
                }
                Err(e) => {
                    warn!(error = %e, "Failed to serialize genome to YAML");
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32004, "message": format!("Genome serialization error: {}", e) }
                    })
                }
            },
            Err(e) => {
                warn!(error = %e, "Failed to read genome from SelfField");
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32004, "message": format!("Genome read error: {}", e) }
                })
            }
        }
    }

    pub(super) async fn handle_evolution(
        &self,
        id: &serde_json::Value,
        _request: &serde_json::Value,
    ) -> serde_json::Value {
        // Return recent evolution log entries from episodic memory.
        match self
            .subsystems
            .episodic_memory
            .lock()
            .await
            .recall_evolution_logs(20)
        {
            Ok(entries) => {
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "evolution": entries,
                        "current_version": "0.1.0"
                    }
                })
            }
            Err(e) => {
                warn!(error = %e, "Failed to recall evolution logs");
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32002, "message": format!("Evolution recall error: {}", e) }
                })
            }
        }
    }
}
