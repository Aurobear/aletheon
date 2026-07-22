//! Composition-owned skill reload and prompt-prefix refresh.

use std::sync::Arc;
use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::application::admin_service::{AdminServiceError, SkillAdminPort};
use crate::composition::prefix_builder::PrefixBuilder;

pub struct DefaultSkillAdmin {
    loader: Arc<Mutex<corpus::SkillLoader>>,
    cached_prefix: Arc<Mutex<String>>,
    config_prompt: String,
}

impl DefaultSkillAdmin {
    pub fn new(
        loader: Arc<Mutex<corpus::SkillLoader>>,
        cached_prefix: Arc<Mutex<String>>,
        config_prompt: String,
    ) -> Self {
        Self {
            loader,
            cached_prefix,
            config_prompt,
        }
    }
}

#[async_trait]
impl SkillAdminPort for DefaultSkillAdmin {
    async fn reload(&self) -> Result<usize, AdminServiceError> {
        let count = self.loader.lock().await.reload();
        let new_prefix = {
            let loader = self.loader.lock().await;
            PrefixBuilder::build(
                &self.config_prompt,
                loader.skills(),
            )
        };
        *self.cached_prefix.lock().await = new_prefix;
        Ok(count)
    }
}
