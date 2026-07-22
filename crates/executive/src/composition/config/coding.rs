//! Deployment configuration for the private coding-runtime adapter.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CodingRuntimeConfig {
    #[serde(default)] pub enabled: bool,
    #[serde(default)] pub executable: PathBuf,
    #[serde(default)] pub trusted_executable_dir: Option<PathBuf>,
    #[serde(default)] pub fixed_args: Vec<String>,
    #[serde(default)] pub package_version: String,
    #[serde(default)] pub executable_sha256: String,
    #[serde(default = "default_protocol_version")] pub json_protocol_version: u32,
    #[serde(default)] pub worktree_base: PathBuf,
    #[serde(default = "default_timeout_ms")] pub timeout_ms: u64,
    #[serde(default = "default_max_output_bytes")] pub max_output_bytes: usize,
    #[serde(default)] pub allowed_paths: Vec<PathBuf>,
    #[serde(default)] pub forbidden_paths: Vec<PathBuf>,
    #[serde(default = "default_true")] pub require_namespace_isolation: bool,
    #[serde(default)] pub network_enabled: bool,
}

impl Default for CodingRuntimeConfig {
    fn default() -> Self {
        Self { enabled: false, executable: PathBuf::new(), trusted_executable_dir: None,
            fixed_args: vec![], package_version: String::new(), executable_sha256: String::new(),
            json_protocol_version: default_protocol_version(), worktree_base: PathBuf::new(),
            timeout_ms: default_timeout_ms(), max_output_bytes: default_max_output_bytes(),
            allowed_paths: vec![], forbidden_paths: vec![], require_namespace_isolation: true,
            network_enabled: false }
    }
}

impl fmt::Debug for CodingRuntimeConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CodingRuntimeConfig").field("enabled", &self.enabled)
            .field("executable", &self.executable).field("trusted_executable_dir", &self.trusted_executable_dir)
            .field("fixed_arg_count", &self.fixed_args.len()).field("package_version", &self.package_version)
            .field("json_protocol_version", &self.json_protocol_version).field("worktree_base", &self.worktree_base)
            .field("timeout_ms", &self.timeout_ms).field("max_output_bytes", &self.max_output_bytes)
            .field("allowed_paths", &self.allowed_paths).field("forbidden_paths", &self.forbidden_paths)
            .field("require_namespace_isolation", &self.require_namespace_isolation)
            .field("network_enabled", &self.network_enabled).finish()
    }
}

fn default_true() -> bool { true }
fn default_timeout_ms() -> u64 { 30 * 60 * 1_000 }
fn default_protocol_version() -> u32 { 3 }
fn default_max_output_bytes() -> usize { 1024 * 1024 }
