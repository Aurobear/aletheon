use std::sync::Arc;

use application::turn::ports::TurnSessionPort;
use async_trait::async_trait;
use contracts::SessionAppendStore;

pub struct SessionAppendTurnPort {
    inner: Arc<dyn SessionAppendStore>,
}

impl SessionAppendTurnPort {
    pub fn new(inner: Arc<dyn SessionAppendStore>) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl TurnSessionPort for SessionAppendTurnPort {
    async fn load_session(
        &self,
        id: &contracts::SessionId,
    ) -> anyhow::Result<Option<contracts::SessionRecord>> {
        self.inner.load_session(id).await
    }
    async fn load_items(
        &self,
        session: &contracts::SessionId,
        through_sequence: Option<u64>,
    ) -> anyhow::Result<Vec<contracts::ItemRecord>> {
        self.inner.load_items(session, through_sequence).await
    }
    async fn create(&self, session: contracts::SessionRecord) -> anyhow::Result<()> {
        self.inner.create(session).await
    }
    async fn bind_principal(
        &self,
        session: &contracts::SessionId,
        principal: &contracts::PrincipalId,
    ) -> anyhow::Result<()> {
        self.inner.bind_principal(session, principal).await
    }
    async fn append(
        &self,
        session: &contracts::SessionId,
        expected_sequence: u64,
        item: contracts::ItemRecord,
    ) -> anyhow::Result<contracts::AppendOutcome> {
        self.inner.append(session, expected_sequence, item).await
    }
}
