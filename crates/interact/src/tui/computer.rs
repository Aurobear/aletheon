//! Typed computer host requests. Interact never constructs or executes Corpus drivers.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComputerHostRequest(pub fabric::protocol::client::ComputerHostParams);

impl ComputerHostRequest {
    pub fn parse(input: &str) -> anyhow::Result<Self> {
        let mut parts = input.split_whitespace();
        let operation = parts
            .next()
            .ok_or_else(|| anyhow::anyhow!("computer operation is required"))?;
        Ok(Self(fabric::protocol::client::ComputerHostParams {
            operation: operation.into(),
            arguments: parts.map(str::to_string).collect(),
        }))
    }

    pub fn to_rpc(&self, id: u64) -> serde_json::Value {
        fabric::protocol::client::ClientRpcRequest::HostComputer(self.0.clone())
            .to_json_rpc(Some(id))
            .expect("typed computer request serializes")
    }
}
