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
        let session_id = self
            .subsystems
            .runtime
            .lock()
            .await
            .config()
            .session_id
            .clone();
        let iteration = self.subsystems.runtime.lock().await.iteration();
        let turn_count = {
            let (_sid, sm_arc) = match self.get_or_create_session(None).await {
                Ok(v) => v,
                Err(e) => {
                    return json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32000, "message": e.to_string() }
                    })
                }
            };
            let tc = sm_arc.lock().await.turn_count();
            tc
        };

        // Reflection and evolution counts from episodic memory
        let reflection_count = self
            .subsystems
            .memory
            .episodic_memory
            .lock()
            .await
            .reflection_count()
            .unwrap_or(0);
        let evolution_count = self
            .subsystems
            .memory
            .episodic_memory
            .lock()
            .await
            .evolution_log_count()
            .unwrap_or(0);

        // Care weights, boundary rules, and attention from SelfField
        let sf = self.subsystems.self_field.lock().await;
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
        use crate::r#impl::health::ComponentHealth;

        let now = self.turn_orchestrator.clock.mono_now();
        let uptime = now.0.saturating_sub(self.started_at.0) / 1000;
        let active = self
            .active_connections
            .load(std::sync::atomic::Ordering::Relaxed);
        let session_count = self.sessions.lock().await.len();
        let minimum_free_bytes = std::env::var("ALETHEON_MINIMUM_FREE_BYTES")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(5 * 1024 * 1024 * 1024);
        let maximum_backup_age_secs = std::env::var("ALETHEON_MAXIMUM_BACKUP_AGE_SECS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(36 * 60 * 60);
        let backup_required = std::env::var("ALETHEON_BACKUP_REQUIRED")
            .is_ok_and(|value| matches!(value.as_str(), "1" | "true" | "yes"));
        let health_data_root = std::env::var_os("ALETHEON_DATA_ROOT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| self.subsystems.session.data_dir.clone());
        self.health.refresh_storage(
            &health_data_root,
            minimum_free_bytes,
            backup_required,
            maximum_backup_age_secs,
        );

        self.health.set(
            "telegram",
            match &self.telegram_task {
                Some(task) if task.is_finished() => ComponentHealth::degraded("worker_stopped"),
                Some(_) => ComponentHealth::ready(),
                None => ComponentHealth::disabled(),
            },
        );
        self.health.set(
            "google_sync",
            match &self.google_sync {
                Some(sync) if sync.lock().await.is_some() => ComponentHealth::ready(),
                Some(_) => ComponentHealth::degraded("worker_stopped"),
                None => ComponentHealth::disabled(),
            },
        );
        let supplemental = self
            .subsystems
            .memory
            .supplemental_memory_health
            .lock()
            .unwrap()
            .clone();
        self.health.set(
            "gbrain_spool",
            if !supplemental.supplemental_enabled {
                ComponentHealth::disabled()
            } else if supplemental.degraded {
                let mut health = ComponentHealth::degraded("supplemental_memory");
                health.count = Some(supplemental.queue_depth as u64);
                health
            } else {
                let mut health = ComponentHealth::ready();
                health.count = Some(supplemental.queue_depth as u64);
                health
            },
        );
        if self
            .daemon_cancel_token
            .as_ref()
            .is_some_and(tokio_util::sync::CancellationToken::is_cancelled)
        {
            self.health.begin_shutdown();
        }
        let production = self.health.snapshot();
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "status": production.readiness,
                "liveness": production.liveness,
                "readiness": production.readiness,
                "components": production.components,
                "uptime_seconds": uptime,
                "active_connections": active,
                "session_count": session_count,
                "daemon_version": env!("CARGO_PKG_VERSION")
            }
        })
    }
}
