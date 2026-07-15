//! Corpus group — tools, skills, hooks.

use std::sync::Arc;

use tokio::sync::Mutex;

use corpus::tools::tools::ToolRegistry;
use corpus::HookRegistry;
use corpus::SkillLoader;
use corpus::SkillRouter;

use crate::core::config::HooksConfig;

pub type ToolRegistryHandle = Arc<Mutex<ToolRegistry>>;
pub type HookRegistryHandle = Arc<Mutex<HookRegistry>>;

pub struct CorpusGroup {
    pub tools: ToolRegistryHandle,
    pub skill_loader: Arc<Mutex<SkillLoader>>,
    pub skill_router: Arc<Mutex<SkillRouter>>,
    pub hook_registry: HookRegistryHandle,
    pub hooks_config: HooksConfig,
}
