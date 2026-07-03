//! Workflow persistence and execution RPC handlers.
//!
//! Methods: workflow.save, workflow.load, workflow.list, workflow.delete, workflow.run.

use super::RequestHandler;

use serde_json::json;

use crate::r#impl::orchestration::digraph::graph::{DiGraph, WorkflowDef};
use crate::r#impl::orchestration::digraph::state::GraphState;
use crate::r#impl::orchestration::store::WorkflowStore;

impl RequestHandler {
    pub(super) async fn handle_workflow_save(
        &self,
        id: &serde_json::Value,
        request: &serde_json::Value,
    ) -> serde_json::Value {
        let name = request["params"]["name"].as_str().unwrap_or("");
        if name.is_empty() {
            return json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32602, "message": "Missing 'name' parameter" }
            });
        }
        match serde_json::from_value::<WorkflowDef>(request["params"]["def"].clone()) {
            Ok(def) => {
                let graph = DiGraph::from_def(&def);
                match WorkflowStore::new(WorkflowStore::default_dir()) {
                    Ok(store) => match store.save(name, &graph) {
                        Ok(()) => json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": { "ok": true, "name": name }
                        }),
                        Err(e) => json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": { "code": -32040, "message": format!("Save error: {e}") }
                        }),
                    },
                    Err(e) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32040, "message": format!("Store init error: {e}") }
                    }),
                }
            }
            Err(e) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32602, "message": format!("Invalid WorkflowDef: {e}") }
            }),
        }
    }

    pub(super) async fn handle_workflow_load(
        &self,
        id: &serde_json::Value,
        request: &serde_json::Value,
    ) -> serde_json::Value {
        let name = request["params"]["name"].as_str().unwrap_or("");
        if name.is_empty() {
            return json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32602, "message": "Missing 'name' parameter" }
            });
        }
        match WorkflowStore::new(WorkflowStore::default_dir()) {
            Ok(store) => match store.load(name) {
                Ok(graph) => {
                    let def = graph.to_def();
                    match serde_json::to_value(&def) {
                        Ok(v) => json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": { "def": v, "name": name }
                        }),
                        Err(e) => json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": { "code": -32040, "message": format!("Serialize error: {e}") }
                        }),
                    }
                }
                Err(e) => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32041, "message": format!("Load error: {e}") }
                }),
            },
            Err(e) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32040, "message": format!("Store init error: {e}") }
            }),
        }
    }

    pub(super) async fn handle_workflow_list(
        &self,
        id: &serde_json::Value,
        _request: &serde_json::Value,
    ) -> serde_json::Value {
        match WorkflowStore::new(WorkflowStore::default_dir()) {
            Ok(store) => match store.list() {
                Ok(names) => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "names": names }
                }),
                Err(e) => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32040, "message": format!("List error: {e}") }
                }),
            },
            Err(e) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32040, "message": format!("Store init error: {e}") }
            }),
        }
    }

    pub(super) async fn handle_workflow_delete(
        &self,
        id: &serde_json::Value,
        request: &serde_json::Value,
    ) -> serde_json::Value {
        let name = request["params"]["name"].as_str().unwrap_or("");
        if name.is_empty() {
            return json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32602, "message": "Missing 'name' parameter" }
            });
        }
        match WorkflowStore::new(WorkflowStore::default_dir()) {
            Ok(store) => match store.delete(name) {
                Ok(()) => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "ok": true, "name": name }
                }),
                Err(e) => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32040, "message": format!("Delete error: {e}") }
                }),
            },
            Err(e) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32040, "message": format!("Store init error: {e}") }
            }),
        }
    }

    pub(super) async fn handle_workflow_run(
        &self,
        id: &serde_json::Value,
        request: &serde_json::Value,
    ) -> serde_json::Value {
        let name = request["params"]["name"].as_str().unwrap_or("");
        if name.is_empty() {
            return json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32602, "message": "Missing 'name' parameter" }
            });
        }
        match WorkflowStore::new(WorkflowStore::default_dir()) {
            Ok(store) => {
                let registry = &*self.agent_registry;
                match store.run(name, registry, GraphState::new()).await {
                    Ok(state) => match serde_json::to_value(&state) {
                        Ok(v) => json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": { "state": v, "name": name }
                        }),
                        Err(e) => json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": { "code": -32040, "message": format!("Serialize error: {e}") }
                        }),
                    },
                    Err(e) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32042, "message": format!("Run error: {e}") }
                    }),
                }
            }
            Err(e) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32040, "message": format!("Store init error: {e}") }
            }),
        }
    }
}
