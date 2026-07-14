//! Goal/objective tracking RPC handlers.
//!
//! Methods: goal.set, goal.show, goal.status, goal.resume.

use super::RequestHandler;

use serde_json::json;

impl RequestHandler {
    pub(super) async fn handle_goal_set(
        &self,
        id: &serde_json::Value,
        request: &serde_json::Value,
    ) -> serde_json::Value {
        let p = &request["params"];
        let description = p["description"].as_str().unwrap_or("");
        let scope = p["scope"].as_str().unwrap_or("session");
        let session_id = match self.get_or_create_session(None).await {
            Ok(v) => v.0,
            Err(e) => {
                return json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32000, "message": e.to_string() }
                })
            }
        };
        let store = self.subsystems.memory.objective_store.lock().await;
        match store.create(description, None, &session_id, scope) {
            Ok(oid) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "objective_id": oid }
            }),
            Err(e) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32020, "message": e.to_string() }
            }),
        }
    }

    pub(super) async fn handle_goal_show(
        &self,
        id: &serde_json::Value,
        request: &serde_json::Value,
    ) -> serde_json::Value {
        let oid = request["params"]["id"].as_i64().unwrap_or(0);
        let store = self.subsystems.memory.objective_store.lock().await;
        match store.get(oid) {
            Ok(Some(obj)) => {
                let subs = store.sub_goals(oid).unwrap_or_default();
                let summaries: Vec<_> = subs.iter().map(|s| s.to_summary()).collect();
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "objective": obj, "sub_goals": summaries }
                })
            }
            Ok(None) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32021, "message": "objective not found" }
            }),
            Err(e) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32020, "message": e.to_string() }
            }),
        }
    }

    pub(super) async fn handle_goal_status(
        &self,
        id: &serde_json::Value,
        request: &serde_json::Value,
    ) -> serde_json::Value {
        let p = &request["params"];
        let store = self.subsystems.memory.objective_store.lock().await;
        // With an id: update status. Without: list objectives.
        if let Some(oid) = p["id"].as_i64() {
            let new_status = p["status"].as_str().unwrap_or("in_progress");
            match store.set_status(oid, new_status) {
                Ok(changed) => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "ok": changed }
                }),
                Err(e) => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32020, "message": e.to_string() }
                }),
            }
        } else {
            let filter = p["filter"].as_str();
            match store.list(filter, 50) {
                Ok(rows) => {
                    let summaries: Vec<_> = rows.iter().map(|r| r.to_summary()).collect();
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": { "objectives": summaries }
                    })
                }
                Err(e) => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32020, "message": e.to_string() }
                }),
            }
        }
    }

    pub(super) async fn handle_goal_resume(
        &self,
        id: &serde_json::Value,
        _request: &serde_json::Value,
    ) -> serde_json::Value {
        let store = self.subsystems.memory.objective_store.lock().await;
        match store.resume() {
            Ok(Some((obj, subs))) => {
                let summaries: Vec<_> = subs.iter().map(|s| s.to_summary()).collect();
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "objective": obj, "sub_goals": summaries }
                })
            }
            Ok(None) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "objective": null, "sub_goals": [] }
            }),
            Err(e) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32020, "message": e.to_string() }
            }),
        }
    }

    // -----------------------------------------------------------------------
    // M2 Goal lifecycle RPC (new — beside legacy API)
    // -----------------------------------------------------------------------

    pub(super) async fn handle_goal_create(
        &self,
        id: &serde_json::Value,
        request: &serde_json::Value,
    ) -> serde_json::Value {
        let p = &request["params"];
        let intent = p["intent"].as_str().unwrap_or("");
        if intent.is_empty() {
            return json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32602, "message": "intent must not be empty" }
            });
        }
        let scope = p["scope"].as_str().unwrap_or("session");
        let session_id = match self.get_or_create_session(None).await {
            Ok(v) => v.0,
            Err(e) => {
                return json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32000, "message": e.to_string() }
                })
            }
        };
        let spec = fabric::GoalSpec {
            original_intent: intent.to_string(),
            desired_state: vec![],
            constraints: vec![],
            acceptance_criteria: vec![],
            budget: Default::default(),
        };
        let store = self.subsystems.memory.objective_store.lock().await;
        match store.create_goal(
            &fabric::PrincipalId(session_id.clone()),
            &session_id,
            scope,
            &spec,
        ) {
            Ok(snapshot) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "id": snapshot.id.0,
                    "state": snapshot.state.as_str(),
                    "intent": snapshot.spec.original_intent,
                    "version": snapshot.version,
                }
            }),
            Err(e) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32020, "message": e.to_string() }
            }),
        }
    }

    pub(super) async fn handle_goal_list(
        &self,
        id: &serde_json::Value,
        request: &serde_json::Value,
    ) -> serde_json::Value {
        let p = &request["params"];
        let limit = p["limit"].as_u64().unwrap_or(20).min(100) as usize;
        let store = self.subsystems.memory.objective_store.lock().await;
        match store.list_goals(&[], limit) {
            Ok(snapshots) => {
                let items: Vec<_> = snapshots
                    .iter()
                    .map(|s| {
                        json!({
                            "id": s.id.0,
                            "state": s.state.as_str(),
                            "intent": s.spec.original_intent,
                            "version": s.version,
                        })
                    })
                    .collect();
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "goals": items }
                })
            }
            Err(e) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32020, "message": e.to_string() }
            }),
        }
    }

    pub(super) async fn handle_goal_pause(
        &self,
        id: &serde_json::Value,
        request: &serde_json::Value,
    ) -> serde_json::Value {
        let goal_id = match request["params"]["goal_id"].as_i64() {
            Some(v) if v > 0 => fabric::GoalId(v),
            _ => {
                return json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32602, "message": "invalid goal_id" }
                });
            }
        };
        let store = self.subsystems.memory.objective_store.lock().await;
        let current = match store.get_goal(goal_id) {
            Ok(Some(g)) => g,
            Ok(None) => {
                return json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32021, "message": "goal not found" }
                });
            }
            Err(e) => {
                return json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32020, "message": e.to_string() }
                });
            }
        };
        match store.transition_goal(
            goal_id,
            current.version,
            fabric::GoalState::Suspended,
            None,
            &serde_json::json!({"action": "pause"}),
        ) {
            Ok(snapshot) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "id": snapshot.id.0,
                    "state": snapshot.state.as_str(),
                    "version": snapshot.version,
                }
            }),
            Err(e) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32022, "message": e.to_string() }
            }),
        }
    }

    pub(super) async fn handle_goal_run(
        &self,
        id: &serde_json::Value,
        request: &serde_json::Value,
    ) -> serde_json::Value {
        let goal_id = match request["params"]["goal_id"].as_i64() {
            Some(v) if v > 0 => fabric::GoalId(v),
            _ => {
                return json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32602, "message": "invalid goal_id" }
                });
            }
        };
        let store = self.subsystems.memory.objective_store.lock().await;
        let current = match store.get_goal(goal_id) {
            Ok(Some(g)) => g,
            Ok(None) => {
                return json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32021, "message": "goal not found" }
                });
            }
            Err(e) => {
                return json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32020, "message": e.to_string() }
                });
            }
        };
        // Resume: map Suspended/Blocked to Ready.
        let next = match current.state {
            fabric::GoalState::Suspended | fabric::GoalState::Blocked => fabric::GoalState::Ready,
            other => {
                return json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32022, "message": format!("cannot run from state {}", other) }
                });
            }
        };
        match store.transition_goal(
            goal_id,
            current.version,
            next,
            None,
            &serde_json::json!({"action": "run"}),
        ) {
            Ok(snapshot) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "id": snapshot.id.0,
                    "state": snapshot.state.as_str(),
                    "version": snapshot.version,
                }
            }),
            Err(e) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32022, "message": e.to_string() }
            }),
        }
    }

    pub(super) async fn handle_goal_cancel(
        &self,
        id: &serde_json::Value,
        request: &serde_json::Value,
    ) -> serde_json::Value {
        let goal_id = match request["params"]["goal_id"].as_i64() {
            Some(v) if v > 0 => fabric::GoalId(v),
            _ => {
                return json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32602, "message": "invalid goal_id" }
                });
            }
        };
        let store = self.subsystems.memory.objective_store.lock().await;
        let current = match store.get_goal(goal_id) {
            Ok(Some(g)) => g,
            Ok(None) => {
                return json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32021, "message": "goal not found" }
                });
            }
            Err(e) => {
                return json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32020, "message": e.to_string() }
                });
            }
        };
        if current.state.is_terminal() {
            return json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32022, "message": "goal already terminal" }
            });
        }
        match store.transition_goal(
            goal_id,
            current.version,
            fabric::GoalState::Cancelled,
            None,
            &serde_json::json!({"action": "cancel"}),
        ) {
            Ok(snapshot) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "id": snapshot.id.0,
                    "state": snapshot.state.as_str(),
                    "version": snapshot.version,
                }
            }),
            Err(e) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32022, "message": e.to_string() }
            }),
        }
    }
}
