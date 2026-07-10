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
        let session_id = self.get_or_create_session(None).await.0;
        let store = self.subsystems.objective_store.lock().await;
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
        let store = self.subsystems.objective_store.lock().await;
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
        let store = self.subsystems.objective_store.lock().await;
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
        let store = self.subsystems.objective_store.lock().await;
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
}
