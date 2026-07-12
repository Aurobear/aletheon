//! Corpus group — tools, skills, hooks.

use std::sync::Arc;

use tokio::sync::Mutex;

use corpus::tools::tools::ToolRegistry;
use corpus::HookRegistry;
use corpus::SkillLoader;
use corpus::SkillRouter;

use crate::core::config::HooksConfig;

pub struct CorpusGroup {
    pub tools: Arc<Mutex<ToolRegistry>>,
    pub skill_loader: Arc<Mutex<SkillLoader>>,
    pub skill_router: Arc<Mutex<SkillRouter>>,
    pub hook_registry: Arc<Mutex<HookRegistry>>,
    pub hooks_config: HooksConfig,
}
