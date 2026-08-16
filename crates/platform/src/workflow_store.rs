//! Filesystem adapter for declarative Application workflow definitions.

use application::workflow::WorkflowDef;
use std::path::{Path, PathBuf};

pub struct WorkflowStore {
    dir: PathBuf,
}

impl WorkflowStore {
    pub fn new(dir: impl AsRef<Path>) -> std::io::Result<Self> {
        let dir = dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&dir)?;
        Ok(Self { dir })
    }

    fn path_for(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{name}.json"))
    }

    pub fn save(&self, name: &str, definition: &WorkflowDef) -> anyhow::Result<()> {
        std::fs::write(
            self.path_for(name),
            serde_json::to_string_pretty(definition)?,
        )?;
        Ok(())
    }

    pub fn load(&self, name: &str) -> anyhow::Result<WorkflowDef> {
        let text = std::fs::read_to_string(self.path_for(name))
            .map_err(|error| anyhow::anyhow!("workflow '{name}' not found: {error}"))?;
        Ok(serde_json::from_str(&text)?)
    }

    pub fn list(&self) -> anyhow::Result<Vec<String>> {
        let mut names = Vec::new();
        for entry in std::fs::read_dir(&self.dir)? {
            let path = entry?.path();
            if path.extension().and_then(|value| value.to_str()) == Some("json") {
                if let Some(name) = path.file_stem().and_then(|value| value.to_str()) {
                    names.push(name.to_owned());
                }
            }
        }
        names.sort();
        Ok(names)
    }

    pub fn delete(&self, name: &str) -> anyhow::Result<()> {
        let path = self.path_for(name);
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        Ok(())
    }
}
