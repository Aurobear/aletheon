use serde::{Deserialize, Serialize};

/// A named entity extracted from memory content.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Entity {
    pub id: EntityId,
    pub name: String,
    pub aliases: Vec<String>,
    pub entity_type: EntityType,
    /// Which MemoryRecord produced this entity.
    pub provenance: String,
}

impl Entity {
    pub fn new(name: String, entity_type: EntityType, provenance: String) -> Self {
        let id = EntityId::derive(&name);
        Self {
            id,
            name,
            aliases: Vec::new(),
            entity_type,
            provenance,
        }
    }

    pub fn with_alias(mut self, alias: String) -> Self {
        self.aliases.push(alias);
        self
    }

    pub fn matches_name(&self, name: &str) -> bool {
        self.name.eq_ignore_ascii_case(name)
            || self.aliases.iter().any(|a| a.eq_ignore_ascii_case(name))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityType {
    Person,
    Company,
    Project,
    Technology,
    Concept,
    Place,
    Organization,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EntityId(pub String);

impl EntityId {
    /// Derive a stable ID from the entity name.
    pub fn derive(name: &str) -> Self {
        let slug = name
            .to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '_' })
            .collect::<String>();
        Self(slug)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_matches_name_case_insensitive() {
        let entity = Entity::new("Alice Chen".into(), EntityType::Person, "rec-1".into());
        assert!(entity.matches_name("alice chen"));
        assert!(!entity.matches_name("Bob"));
    }

    #[test]
    fn entity_matches_alias() {
        let entity = Entity::new("Acme Corp".into(), EntityType::Company, "rec-1".into())
            .with_alias("Acme".into());
        assert!(entity.matches_name("Acme"));
        assert!(entity.matches_name("ACME"));
    }

    #[test]
    fn entity_id_is_stable() {
        let id1 = EntityId::derive("Alice Chen");
        let id2 = EntityId::derive("Alice Chen");
        assert_eq!(id1, id2);
    }
}
