//! Health and status RPC handlers.
//!
//! Methods: status, health.

use super::RequestHandler;

use serde_json::json;

impl RequestHandler {
    pub(super) async fn handle_status(
        &self,
        id: &serde_json::Value,
        _request: &serde_json::Value,
    ) -> serde_json::Value {
        let state = self.state.lock().await;
        let session_id = state.runtime.config().session_id.clone();
        let iteration = state.runtime.iteration();
        drop(state);
        let turn_count = {
            let (_sid, sm_arc) = self.get_or_create_session(None).await;
            let tc = sm_arc.lock().await.turn_count();
            tc
        };

        // Reflection and evolution counts from episodic memory
        let reflection_count = self
            .episodic_memory
            .lock()
            .await
            .reflection_count()
            .unwrap_or(0);
        let evolution_count = self
            .episodic_memory
            .lock()
            .await
            .evolution_log_count()
            .unwrap_or(0);

        // Care weights, boundary rules, and attention from SelfField
        let sf = self.self_field.lock().await;
        let care_weights: Vec<serde_json::Value> = sf
            .care()
            .all_cares()
            .into_iter()
            .map(|c| json!({ "topic": c.topic, "weight": c.weight }))
            .collect();
        let boundary_total = sf.boundary().rule_count();
        let boundary_immutable = sf.boundary().immutable_rule_count();
        let attention_focus = sf
            .attention()
            .current_focus()
            .map(|f| f.topic)
            .unwrap_or_default();
        drop(sf);

        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "status": {
                    "session_id": session_id,
                    "turn_count": turn_count,
                    "iteration": iteration,
                    "reflection_count": reflection_count,
                    "evolution_count": evolution_count,
                    "care_weights": care_weights,
                    "boundary_rules": boundary_total,
                    "boundary_immutable": boundary_immutable,
                    "attention_focus": attention_focus,
                }
            }
        })
    }

    pub(super) async fn handle_health(
        &self,
        id: &serde_json::Value,
        _request: &serde_json::Value,
    ) -> serde_json::Value {
        let uptime = self.started_at.elapsed().as_secs();
        let active = self
            .active_connections
            .load(std::sync::atomic::Ordering::Relaxed);
        let session_count = self.sessions.lock().await.len();
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "status": "ok",
                "uptime_seconds": uptime,
                "active_connections": active,
                "session_count": session_count,
                "daemon_version": "0.1.0"
            }
        })
    }
}
