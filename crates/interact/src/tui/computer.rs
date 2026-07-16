//! Typed computer host requests. Interact never constructs or executes Corpus drivers.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComputerHostRequest {
    pub operation: String,
    pub arguments: Vec<String>,
}

impl ComputerHostRequest {
    pub fn parse(input: &str) -> anyhow::Result<Self> {
        let mut parts = input.split_whitespace();
        let operation = parts
            .next()
            .ok_or_else(|| anyhow::anyhow!("computer operation is required"))?;
        Ok(Self {
            operation: operation.into(),
            arguments: parts.map(str::to_string).collect(),
        })
    }

    pub fn to_rpc(&self, id: u64) -> serde_json::Value {
        serde_json::json!({"jsonrpc":"2.0", "id":id, "method":"host.computer", "params":self})
    }
}
