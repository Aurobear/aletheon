//! Once-bound port slot for delayed injection of a conscious context reader.
//!
//! The slot is created before the recurrent workspace exists and is injected
//! into SelfField and context assembly. The registry binds itself after
//! Dasein construction, guaranteeing that every code path either sees the
//! bound reader or fails with a clear "not yet bound" error.

use async_trait::async_trait;
use fabric::LatestConsciousContextPort;
use fabric::{AgoraSpaceId, ConsciousContextProjection};
use std::sync::Arc;

#[derive(Default)]
pub struct ConsciousContextSlot {
    inner: parking_lot::RwLock<Option<Arc<dyn LatestConsciousContextPort>>>,
}

impl ConsciousContextSlot {
    pub fn bind(&self, reader: Arc<dyn LatestConsciousContextPort>) -> anyhow::Result<()> {
        let mut inner = self.inner.write();
        anyhow::ensure!(inner.is_none(), "conscious context reader already bound");
        *inner = Some(reader);
        Ok(())
    }
}

#[async_trait]
impl LatestConsciousContextPort for ConsciousContextSlot {
    async fn latest_context(
        &self,
        space: &AgoraSpaceId,
    ) -> anyhow::Result<ConsciousContextProjection> {
        let reader = self.inner.read().clone();
        let reader =
            reader.ok_or_else(|| anyhow::anyhow!("conscious context reader not yet bound"))?;
        reader.latest_context(space).await
    }
}
