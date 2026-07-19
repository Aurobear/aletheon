use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemaVersion(pub u32);

pub struct MigrationCoordinator {
    versions: Vec<(SchemaVersion, String)>,
}

impl MigrationCoordinator {
    pub fn new() -> Self { Self { versions: vec![] } }

    pub fn register(&mut self, version: u32, description: &str) {
        self.versions.push((SchemaVersion(version), description.into()));
    }

    pub fn current(&self) -> SchemaVersion {
        self.versions.last().map(|(v, _)| v.clone()).unwrap_or(SchemaVersion(0))
    }

    pub fn needs_migration(&self, from: &SchemaVersion) -> bool {
        self.versions.iter().any(|(v, _)| v.0 > from.0)
    }

    pub fn path(&self, from: &SchemaVersion, to: &SchemaVersion) -> Vec<&str> {
        self.versions.iter()
            .filter(|(v, _)| v.0 > from.0 && v.0 <= to.0)
            .map(|(_, desc)| desc.as_str())
            .collect()
    }
}

impl Default for MigrationCoordinator {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_path_between_versions() {
        let mut c = MigrationCoordinator::new();
        c.register(1, "add turn table");
        c.register(2, "add tool_call table");
        c.register(3, "add checkpoint table");
        assert_eq!(c.current().0, 3);
        assert!(c.needs_migration(&SchemaVersion(1)));
        let path = c.path(&SchemaVersion(1), &SchemaVersion(3));
        assert_eq!(path.len(), 2);
    }
}
