//! Workflow persistence RPC handlers.

use super::RequestHandler;
use crate::r#impl::orchestration::digraph::graph::WorkflowDef;
use serde_json::json;

impl RequestHandler {
    pub(super) async fn handle_workflow_save(
        &self,
        id: &serde_json::Value,
        request: &serde_json::Value,
    ) -> serde_json::Value {
        let name = request["params"]["name"].as_str().unwrap_or("");
        if name.is_empty() {
            return missing_name(id);
        }
        let definition = match serde_json::from_value::<WorkflowDef>(
            request["params"]["def"].clone(),
        ) {
            Ok(definition) => definition,
            Err(error) => {
                return json!({"jsonrpc":"2.0","id":id,"error":{"code":-32602,"message":format!("Invalid WorkflowDef: {error}")}})
            }
        };
        match self.ports.workflow.save(name.into(), definition).await {
            Ok(()) => json!({"jsonrpc":"2.0","id":id,"result":{"ok":true,"name":name}}),
            Err(error) => operation_error(id, "Save", error),
        }
    }

    pub(super) async fn handle_workflow_load(
        &self,
        id: &serde_json::Value,
        request: &serde_json::Value,
    ) -> serde_json::Value {
        let name = request["params"]["name"].as_str().unwrap_or("");
        if name.is_empty() {
            return missing_name(id);
        }
        match self.ports.workflow.load(name.into()).await {
            Ok(definition) => {
                json!({"jsonrpc":"2.0","id":id,"result":{"def":definition,"name":name}})
            }
            Err(error) => operation_error(id, "Load", error),
        }
    }

    pub(super) async fn handle_workflow_list(
        &self,
        id: &serde_json::Value,
        _request: &serde_json::Value,
    ) -> serde_json::Value {
        match self.ports.workflow.list().await {
            Ok(names) => json!({"jsonrpc":"2.0","id":id,"result":{"names":names}}),
            Err(error) => operation_error(id, "List", error),
        }
    }

    pub(super) async fn handle_workflow_delete(
        &self,
        id: &serde_json::Value,
        request: &serde_json::Value,
    ) -> serde_json::Value {
        let name = request["params"]["name"].as_str().unwrap_or("");
        if name.is_empty() {
            return missing_name(id);
        }
        match self.ports.workflow.delete(name.into()).await {
            Ok(()) => json!({"jsonrpc":"2.0","id":id,"result":{"ok":true,"name":name}}),
            Err(error) => operation_error(id, "Delete", error),
        }
    }

    pub(super) async fn handle_workflow_run(
        &self,
        id: &serde_json::Value,
        request: &serde_json::Value,
    ) -> serde_json::Value {
        let name = request["params"]["name"].as_str().unwrap_or("");
        if name.is_empty() {
            return missing_name(id);
        }
        json!({"jsonrpc":"2.0","id":id,"error":{"code":-32042,"message":"workflow.run not implemented — agent_registry parked (multi-agent unwired)"}})
    }
}

fn missing_name(id: &serde_json::Value) -> serde_json::Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":-32602,"message":"Missing 'name' parameter"}})
}

fn operation_error(
    id: &serde_json::Value,
    operation: &str,
    error: anyhow::Error,
) -> serde_json::Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":-32040,"message":format!("{operation} error: {error}")}})
}
