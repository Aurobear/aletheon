//! Goal/objective tracking RPC handlers.

use super::RequestHandler;

use crate::service::goal_service::{GoalAction, GoalServiceError};
use serde_json::json;

fn goal_error(
    id: &serde_json::Value,
    error: GoalServiceError,
    transition: bool,
) -> serde_json::Value {
    let code = match error {
        GoalServiceError::NotFound => -32021,
        GoalServiceError::InvalidTransition(_) | GoalServiceError::Conflict(_) => -32022,
        GoalServiceError::Store(_) if transition => -32022,
        GoalServiceError::Store(_) => -32020,
    };
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": error.to_string() }
    })
}

fn goal_id(
    id: &serde_json::Value,
    request: &serde_json::Value,
) -> Result<fabric::GoalId, serde_json::Value> {
    request["params"]["goal_id"]
        .as_i64()
        .filter(|value| *value > 0)
        .map(fabric::GoalId)
        .ok_or_else(|| {
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32602, "message": "invalid goal_id" }
            })
        })
}

impl RequestHandler {
    pub(super) async fn handle_goal_set(
        &self,
        id: &serde_json::Value,
        request: &serde_json::Value,
    ) -> serde_json::Value {
        let params = &request["params"];
        let description = params["description"].as_str().unwrap_or("");
        let scope = params["scope"].as_str().unwrap_or("session");
        let session_id = match self.ports.sessions.current().await {
            Ok(value) => value.session_id,
            Err(error) => {
                return json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32000, "message": error.to_string() }
                })
            }
        };
        match self
            .ports
            .goals
            .create_legacy(description.into(), session_id, scope.into())
            .await
        {
            Ok(objective_id) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "objective_id": objective_id }
            }),
            Err(error) => goal_error(id, error, false),
        }
    }

    pub(super) async fn handle_goal_show(
        &self,
        id: &serde_json::Value,
        request: &serde_json::Value,
    ) -> serde_json::Value {
        let objective_id = request["params"]["id"].as_i64().unwrap_or(0);
        match self.ports.goals.show_legacy(objective_id).await {
            Ok(detail) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "objective": detail.objective,
                    "sub_goals": detail.sub_goals,
                }
            }),
            Err(GoalServiceError::NotFound) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32021, "message": "objective not found" }
            }),
            Err(error) => goal_error(id, error, false),
        }
    }

    pub(super) async fn handle_goal_status(
        &self,
        id: &serde_json::Value,
        request: &serde_json::Value,
    ) -> serde_json::Value {
        let params = &request["params"];
        if let Some(objective_id) = params["id"].as_i64() {
            let status = params["status"].as_str().unwrap_or("in_progress");
            match self
                .ports
                .goals
                .set_legacy_status(objective_id, status.into())
                .await
            {
                Ok(changed) => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "ok": changed }
                }),
                Err(error) => goal_error(id, error, false),
            }
        } else {
            let filter = params["filter"].as_str().map(str::to_string);
            match self.ports.goals.list_legacy(filter).await {
                Ok(objectives) => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "objectives": objectives }
                }),
                Err(error) => goal_error(id, error, false),
            }
        }
    }

    pub(super) async fn handle_goal_resume(
        &self,
        id: &serde_json::Value,
        _request: &serde_json::Value,
    ) -> serde_json::Value {
        match self.ports.goals.resume_legacy().await {
            Ok(Some(resume)) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "objective": resume.objective,
                    "sub_goals": resume.sub_goals,
                }
            }),
            Ok(None) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "objective": null, "sub_goals": [] }
            }),
            Err(error) => goal_error(id, error, false),
        }
    }

    pub(super) async fn handle_goal_create(
        &self,
        id: &serde_json::Value,
        request: &serde_json::Value,
    ) -> serde_json::Value {
        let params = &request["params"];
        let intent = params["intent"].as_str().unwrap_or("");
        if intent.is_empty() {
            return json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32602, "message": "intent must not be empty" }
            });
        }
        let scope = params["scope"].as_str().unwrap_or("session");
        let session_id = match self.ports.sessions.current().await {
            Ok(value) => value.session_id,
            Err(error) => {
                return json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32000, "message": error.to_string() }
                })
            }
        };
        let spec = fabric::GoalSpec {
            original_intent: intent.into(),
            desired_state: vec![],
            constraints: vec![],
            acceptance_criteria: vec![],
            budget: Default::default(),
        };
        match self
            .ports
            .goals
            .create_goal(
                fabric::PrincipalId(session_id.clone()),
                session_id,
                scope.into(),
                spec,
            )
            .await
        {
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
            Err(error) => goal_error(id, error, false),
        }
    }

    pub(super) async fn handle_goal_list(
        &self,
        id: &serde_json::Value,
        request: &serde_json::Value,
    ) -> serde_json::Value {
        let limit = request["params"]["limit"].as_u64().unwrap_or(20).min(100) as usize;
        match self.ports.goals.list_goals(limit).await {
            Ok(snapshots) => {
                let goals: Vec<_> = snapshots
                    .into_iter()
                    .map(|snapshot| {
                        json!({
                            "id": snapshot.id.0,
                            "state": snapshot.state.as_str(),
                            "intent": snapshot.spec.original_intent,
                            "version": snapshot.version,
                        })
                    })
                    .collect();
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "goals": goals }
                })
            }
            Err(error) => goal_error(id, error, false),
        }
    }

    async fn handle_goal_action(
        &self,
        id: &serde_json::Value,
        request: &serde_json::Value,
        action: GoalAction,
    ) -> serde_json::Value {
        let goal_id = match goal_id(id, request) {
            Ok(goal_id) => goal_id,
            Err(response) => return response,
        };
        match self.ports.goals.act(goal_id, action, None).await {
            Ok(snapshot) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "id": snapshot.id.0,
                    "state": snapshot.state.as_str(),
                    "version": snapshot.version,
                }
            }),
            Err(error) => goal_error(id, error, true),
        }
    }

    pub(super) async fn handle_goal_pause(
        &self,
        id: &serde_json::Value,
        request: &serde_json::Value,
    ) -> serde_json::Value {
        self.handle_goal_action(id, request, GoalAction::Pause)
            .await
    }

    pub(super) async fn handle_goal_run(
        &self,
        id: &serde_json::Value,
        request: &serde_json::Value,
    ) -> serde_json::Value {
        self.handle_goal_action(id, request, GoalAction::Run).await
    }

    pub(super) async fn handle_goal_cancel(
        &self,
        id: &serde_json::Value,
        request: &serde_json::Value,
    ) -> serde_json::Value {
        self.handle_goal_action(id, request, GoalAction::Cancel)
            .await
    }
}
