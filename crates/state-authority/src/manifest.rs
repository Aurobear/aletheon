use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum StoreKind { Sqlite, JsonFile, InMemory }

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum StoreRole { Authority, Projection, Cache, Legacy }

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AuthorityId(pub String);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StorageManifest {
    pub authority: AuthorityId,
    pub kind: StoreKind,
    pub role: StoreRole,
    pub schema_version: u32,
    pub path: String,
    pub fact_types: Vec<String>,
}

impl StorageManifest {
    pub fn new(id: &str, kind: StoreKind, role: StoreRole, schema_version: u32, path: &str) -> Self {
        Self {
            authority: AuthorityId(id.into()),
            kind,
            role,
            schema_version,
            path: path.into(),
            fact_types: vec![],
        }
    }

    pub fn with_facts(mut self, facts: &[&str]) -> Self {
        self.fact_types = facts.iter().map(|s| s.to_string()).collect();
        self
    }

    pub fn is_authority(&self) -> bool { self.role == StoreRole::Authority }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_distinguishes_authority_from_projection() {
        let auth = StorageManifest::new("events", StoreKind::Sqlite, StoreRole::Authority, 1, "events.db")
            .with_facts(&["turn", "tool_call"]);
        assert!(auth.is_authority());
        assert!(!StorageManifest::new("proj", StoreKind::JsonFile, StoreRole::Projection, 1, "proj.json").is_authority());
    }
}
