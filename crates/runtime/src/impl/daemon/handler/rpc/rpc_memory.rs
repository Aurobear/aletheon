//! Memory (fact store) RPC handlers.
//!
//! Methods: memory.add, memory.list, memory.search, memory.show,
//! memory.forget, memory.pin, memory.unpin.

use super::RequestHandler;

use serde_json::json;

impl RequestHandler {
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
        let fs = self.fact_store.lock().await;
        match fs.add_fact_governed(
            content, "general", tags, scope, "explicit", subject, 0.7, "semantic", 0,
        ) {
            Ok(fact_id) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "fact_id": fact_id }
            }),
            Err(e) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32010, "message": e.to_string() }
            }),
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
        let fs = self.fact_store.lock().await;
        match fs.list_facts(scope, all, 50) {
            Ok(rows) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "facts": rows }
            }),
            Err(e) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32010, "message": e.to_string() }
            }),
        }
    }

    pub(super) async fn handle_memory_search(
        &self,
        id: &serde_json::Value,
        request: &serde_json::Value,
    ) -> serde_json::Value {
        let p = &request["params"];
        let query = p["query"].as_str().unwrap_or("");
        let scope = p["scope"].as_str();
        let fs = self.fact_store.lock().await;
        match fs.search_facts_governed(query, scope, false, 0.15, 20) {
            Ok(rows) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "facts": rows }
            }),
            Err(e) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32010, "message": e.to_string() }
            }),
        }
    }

    pub(super) async fn handle_memory_show(
        &self,
        id: &serde_json::Value,
        request: &serde_json::Value,
    ) -> serde_json::Value {
        let fact_id = request["params"]["id"].as_i64().unwrap_or(0);
        let fs = self.fact_store.lock().await;
        match fs.get_fact(fact_id) {
            Ok(Some(row)) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "fact": row }
            }),
            Ok(None) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32011, "message": "fact not found" }
            }),
            Err(e) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32010, "message": e.to_string() }
            }),
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
        let fs = self.fact_store.lock().await;
        let res = if hard {
            fs.delete_fact(fact_id)
        } else {
            fs.set_status(fact_id, "archived")
        };
        match res {
            Ok(ok) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "ok": ok }
            }),
            Err(e) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32010, "message": e.to_string() }
            }),
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
        let fs = self.fact_store.lock().await;
        match fs.set_pinned(fact_id, pin) {
            Ok(ok) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "ok": ok }
            }),
            Err(e) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32010, "message": e.to_string() }
            }),
        }
    }
}
