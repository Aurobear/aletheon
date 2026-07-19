use crate::events::RuntimeEvent;
use crate::manifest::RuntimeManifest;
use crate::receipt::RuntimeReceipt;
use crate::work_order::WorkOrder;
use async_trait::async_trait;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct PreparedRuntime { pub session_id: String }

#[derive(Clone, Debug)]
pub struct RuntimeHandle { pub id: String }

#[derive(Clone, Debug)]
pub struct RuntimeSnapshot { pub handle: RuntimeHandle, pub state: String }

#[async_trait]
pub trait RuntimeEventSink: Send + Sync {
    async fn emit(&self, event: RuntimeEvent);
}

#[async_trait]
pub trait CapabilityRuntime: Send + Sync {
    fn manifest(&self) -> &RuntimeManifest;
    async fn prepare(&self, order: WorkOrder) -> Result<PreparedRuntime, String>;
    async fn start(&self, prepared: PreparedRuntime, events: Arc<dyn RuntimeEventSink>) -> Result<RuntimeHandle, String>;
    async fn cancel(&self, handle: RuntimeHandle) -> Result<(), String>;
    async fn settle(&self, handle: RuntimeHandle) -> Result<RuntimeReceipt, String>;
}
