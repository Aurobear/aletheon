//! Filesystem-backed path materialization for evaluation scope checks.

use application::evaluation::EvaluationPathResolver;
use std::path::{Path, PathBuf};

pub struct PlatformEvaluationPathResolver;

impl EvaluationPathResolver for PlatformEvaluationPathResolver {
    fn materialize(&self, path: &Path) -> Result<PathBuf, String> {
        let mut missing = Vec::new();
        let mut ancestor = path;
        while !ancestor.exists() {
            let Some(name) = ancestor.file_name() else {
                return Ok(path.to_path_buf());
            };
            missing.push(name.to_os_string());
            let Some(parent) = ancestor.parent() else {
                return Ok(path.to_path_buf());
            };
            ancestor = parent;
        }
        let mut resolved = std::fs::canonicalize(ancestor).map_err(|error| error.to_string())?;
        for component in missing.iter().rev() {
            resolved.push(component);
        }
        Ok(resolved)
    }
}
