use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FactKind {
    Turn,
    ToolCall,
    AgentRun,
    Lease,
    Approval,
    Checkpoint,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DurableFact {
    pub kind: FactKind,
    pub id: String,
    pub sequence: u64,
    pub payload: serde_json::Value,
    pub created_at_ms: i64,
}

pub struct Authority {
    pub fact_kind: FactKind,
    pub store_id: super::manifest::AuthorityId,
}

impl Authority {
    pub fn new(fact_kind: FactKind, store_id: &str) -> Self {
        Self { fact_kind, store_id: super::manifest::AuthorityId(store_id.into()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_fact_has_one_authority() {
        let auth = Authority::new(FactKind::Turn, "events.db");
        assert_eq!(auth.fact_kind, FactKind::Turn);
        assert_eq!(auth.store_id.0, "events.db");
    }
}
