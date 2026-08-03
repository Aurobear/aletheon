//! Authenticated extension package lifecycle RPC handlers.

use fabric::protocol::extension::{
    ExtensionEnableRequestV1, ExtensionPackageIdRequestV1, ExtensionPackagePathRequestV1,
    EXTENSION_PROTOCOL_SCHEMA_V1,
};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use super::RequestHandler;

const INVALID_PARAMS: i64 = -32602;
const EXTENSION_OPERATION_FAILED: i64 = -32060;

impl RequestHandler {
    pub(super) async fn handle_extension_rpc(
        &self,
        connection: &super::super::super::server::ConnectionContext,
        id: &Value,
        request: &Value,
        method: &str,
    ) -> Value {
        let actor = connection.principal_id.0.as_str();
        let result = match method {
            "extension.install" => match params::<ExtensionPackagePathRequestV1>(id, request) {
                Ok(params) => self
                    .ports
                    .extensions
                    .install(actor, &params.path, params.trust_workspace)
                    .await
                    .and_then(json_value),
                Err(response) => return response,
            },
            "extension.enable" => match params::<ExtensionEnableRequestV1>(id, request) {
                Ok(params) => self
                    .ports
                    .extensions
                    .enable(actor, &params.package_id, params.approve_permissions)
                    .await
                    .and_then(json_value),
                Err(response) => return response,
            },
            "extension.disable" => match package_id_params(id, request) {
                Ok(params) => self
                    .ports
                    .extensions
                    .disable(actor, &params.package_id)
                    .await
                    .and_then(json_value),
                Err(response) => return response,
            },
            "extension.upgrade" => match params::<ExtensionPackagePathRequestV1>(id, request) {
                Ok(params) => self
                    .ports
                    .extensions
                    .upgrade(
                        actor,
                        &params.path,
                        params.trust_workspace,
                        params.approve_permissions,
                    )
                    .await
                    .and_then(json_value),
                Err(response) => return response,
            },
            "extension.rollback" => match package_id_params(id, request) {
                Ok(params) => self
                    .ports
                    .extensions
                    .rollback(actor, &params.package_id)
                    .await
                    .and_then(json_value),
                Err(response) => return response,
            },
            "extension.remove" => match package_id_params(id, request) {
                Ok(params) => self
                    .ports
                    .extensions
                    .remove(actor, &params.package_id)
                    .await
                    .and_then(json_value),
                Err(response) => return response,
            },
            "extension.purge" => match package_id_params(id, request) {
                Ok(params) => self
                    .ports
                    .extensions
                    .purge(actor, &params.package_id)
                    .await
                    .and_then(json_value),
                Err(response) => return response,
            },
            "extension.list" => self.ports.extensions.list().and_then(json_value),
            "extension.show" => match package_id_params(id, request) {
                Ok(params) => self
                    .ports
                    .extensions
                    .show(&params.package_id)
                    .and_then(json_value),
                Err(response) => return response,
            },
            "extension.doctor" => match package_id_params(id, request) {
                Ok(params) => self
                    .ports
                    .extensions
                    .doctor(&params.package_id)
                    .and_then(json_value),
                Err(response) => return response,
            },
            _ => unreachable!("dispatcher only routes known extension methods"),
        };
        match result {
            Ok(value) => json!({"jsonrpc":"2.0", "id":id, "result":value}),
            Err(error) => rpc_error(id, EXTENSION_OPERATION_FAILED, format!("{error:#}")),
        }
    }
}

fn json_value<T: serde::Serialize>(value: T) -> anyhow::Result<Value> {
    Ok(serde_json::to_value(value)?)
}

fn package_id_params(id: &Value, request: &Value) -> Result<ExtensionPackageIdRequestV1, Value> {
    let params = params::<ExtensionPackageIdRequestV1>(id, request)?;
    if params.package_id.trim().is_empty() {
        return Err(rpc_error(id, INVALID_PARAMS, "package_id is required"));
    }
    Ok(params)
}

fn params<T: DeserializeOwned + SchemaVersion>(id: &Value, request: &Value) -> Result<T, Value> {
    let params = serde_json::from_value::<T>(request["params"].clone()).map_err(|error| {
        rpc_error(
            id,
            INVALID_PARAMS,
            format!("invalid extension request: {error}"),
        )
    })?;
    if params.schema_version() != EXTENSION_PROTOCOL_SCHEMA_V1 {
        return Err(rpc_error(
            id,
            INVALID_PARAMS,
            format!(
                "unsupported extension schema version {}",
                params.schema_version()
            ),
        ));
    }
    Ok(params)
}

trait SchemaVersion {
    fn schema_version(&self) -> u16;
}

impl SchemaVersion for ExtensionEnableRequestV1 {
    fn schema_version(&self) -> u16 {
        self.schema_version
    }
}

impl SchemaVersion for ExtensionPackagePathRequestV1 {
    fn schema_version(&self) -> u16 {
        self.schema_version
    }
}

impl SchemaVersion for ExtensionPackageIdRequestV1 {
    fn schema_version(&self) -> u16 {
        self.schema_version
    }
}

fn rpc_error(id: &Value, code: i64, message: impl Into<String>) -> Value {
    json!({"jsonrpc":"2.0", "id":id, "error":{"code":code,"message":message.into()}})
}
