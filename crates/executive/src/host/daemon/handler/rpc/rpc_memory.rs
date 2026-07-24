//! Memory (fact store) RPC handlers.
//!
//! Methods: memory.add, memory.list, memory.search, memory.show,
//! memory.forget, memory.pin, memory.unpin.

use super::RequestHandler;

use mnemosyne::{AddFactRequest, FactServiceError, ListFactsRequest, RecallRequest};
use serde_json::json;

fn memory_error(id: &serde_json::Value, error: FactServiceError) -> serde_json::Value {
    let code = if matches!(error, FactServiceError::NotFound) {
        -32011
    } else {
        -32010
    };
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": error.to_string() }
    })
}

impl RequestHandler {
    pub(super) async fn handle_memory_status(&self, id: &serde_json::Value) -> serde_json::Value {
        let health = self
            .ports
            .memory_health
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "memory": {
                    "provider": "composite",
                    "local": "healthy",
                    "supplemental": {
                        "enabled": health.supplemental_enabled,
                        "state": if health.degraded { "degraded" } else { "healthy" },
                        "error_category": health.error_category.map(|value| format!("{value:?}").to_ascii_lowercase()),
                        "queue_depth": health.queue_depth,
                    }
                }
            }
        })
    }

    pub(super) async fn handle_memory_add(
        &self,
        id: &serde_json::Value,
        request: &serde_json::Value,
    ) -> serde_json::Value {
        let p = &request["params"];
        let content = p["content"].as_str().unwrap_or("");
        let scope = p["scope"].as_str().unwrap_or("session");
        let subject = p["subject"].as_str().unwrap_or("");
        let tags = p["tags"].as_str().unwrap_or("");
        match self
            .ports
            .facts
            .add(AddFactRequest {
                content: content.to_string(),
                scope: scope.to_string(),
                subject: subject.to_string(),
                tags: tags.to_string(),
            })
            .await
        {
            Ok(fact_id) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "fact_id": fact_id }
            }),
            Err(error) => memory_error(id, error),
        }
    }

    pub(super) async fn handle_memory_list(
        &self,
        id: &serde_json::Value,
        request: &serde_json::Value,
    ) -> serde_json::Value {
        let p = &request["params"];
        let scope = p["scope"].as_str();
        let all = p["all"].as_bool().unwrap_or(false);
        match self
            .ports
            .facts
            .list(ListFactsRequest {
                scope: scope.map(str::to_string),
                include_archived: all,
            })
            .await
        {
            Ok(rows) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "facts": rows }
            }),
            Err(error) => memory_error(id, error),
        }
    }

    pub(super) async fn handle_memory_search(
        &self,
        id: &serde_json::Value,
        request: &serde_json::Value,
    ) -> serde_json::Value {
        let p = &request["params"];
        let query = p["query"].as_str().unwrap_or("");
        let session = p["scope"].as_str().unwrap_or("global");
        match self
            .ports
            .recall_service
            .recall(RecallRequest::bounded(session, query))
            .await
        {
            Ok(recall) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "facts": recall.items }
            }),
            Err(error) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32010, "message": error.to_string() }
            }),
        }
    }

    pub(super) async fn handle_memory_show(
        &self,
        id: &serde_json::Value,
        request: &serde_json::Value,
    ) -> serde_json::Value {
        let fact_id = request["params"]["id"].as_i64().unwrap_or(0);
        match self.ports.facts.show(fact_id).await {
            Ok(row) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "fact": row }
            }),
            Err(FactServiceError::InvalidInput(_)) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32011, "message": "fact not found" }
            }),
            Err(error) => memory_error(id, error),
        }
    }

    pub(super) async fn handle_memory_forget(
        &self,
        id: &serde_json::Value,
        request: &serde_json::Value,
    ) -> serde_json::Value {
        let p = &request["params"];
        let fact_id = p["id"].as_i64().unwrap_or(0);
        let hard = p["hard"].as_bool().unwrap_or(false);
        match self.ports.facts.forget(fact_id, hard).await {
            Ok(ok) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "ok": ok }
            }),
            Err(error) => memory_error(id, error),
        }
    }

    pub(super) async fn handle_memory_pin(
        &self,
        id: &serde_json::Value,
        request: &serde_json::Value,
        method: &str,
    ) -> serde_json::Value {
        let fact_id = request["params"]["id"].as_i64().unwrap_or(0);
        let pin = method == "memory.pin";
        match self.ports.facts.set_pinned(fact_id, pin).await {
            Ok(ok) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "ok": ok }
            }),
            Err(error) => memory_error(id, error),
        }
    }
}
