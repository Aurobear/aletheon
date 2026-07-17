//! Corpus group — tools, skills, hooks.

use std::sync::Arc;

use tokio::sync::Mutex;

use corpus::tools::tools::ToolRegistry;
use corpus::HookRegistry;

use crate::core::config::HooksConfig;

pub(crate) type ToolRegistryHandle = Arc<Mutex<ToolRegistry>>;
pub(crate) type HookRegistryHandle = Arc<Mutex<HookRegistry>>;

pub(crate) struct CorpusGroup {
    pub(crate) tools: ToolRegistryHandle,
    pub(crate) hook_registry: HookRegistryHandle,
    pub(crate) hooks_config: HooksConfig,
}
