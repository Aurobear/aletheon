//! DaseinEventBridge — bridges EventBus events to DaseinModule.
//!
//! DaseinModule must perceive real system events to exist meaningfully.
//! This bridge subscribes to the central EventBus and translates
//! system events into DaseinEvent messages on the DaseinModule's channel.

use base::event::EventType;
use base::EventBus;
use tokio::sync::mpsc;

use base::dasein::DaseinEvent;

/// Bridges EventBus events to DaseinModule's internal event channel.
///
/// DaseinModule "perceives" the system through this bridge --
/// tool executions, memory storage, evolution triggers, and session
/// lifecycle events all flow into the temporal stream and involvement network.
pub struct DaseinEventBridge {
    dasein_tx: mpsc::Sender<DaseinEvent>,
}

impl DaseinEventBridge {
    pub fn new(dasein_tx: mpsc::Sender<DaseinEvent>) -> Self {
        Self { dasein_tx }
    }

    /// Register subscriptions on the EventBus to forward system events
    /// to the DaseinModule.
    ///
    /// Subscribes to:
    /// - `ToolObservation` -- tool execution results update the involvement network
    /// - `MemoryStored` -- memory events sediment into bewandtnis relations
    /// - `EvolutionTriggered` -- evolution events trigger negativity checks
    /// - `AgentStarted` -- session/lifecycle events update the temporal stream
    pub async fn subscribe(&self, event_bus: &dyn EventBus) -> anyhow::Result<()> {
        let tx = self.dasein_tx.clone();
        event_bus
            .subscribe(
                EventType::ToolObservation,
                Box::new(move |event| {
                    let source = event.source().to_string();
                    let json = event.to_json();
                    let tool_name = json
                        .get("tool_name")
                        .and_then(|v| v.as_str())
                        .unwrap_or(&source);
                    let status = json
                        .get("status")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    let _ = tx.try_send(DaseinEvent::SystemEvent {
                        source: "tool_execution".to_string(),
                        content: format!("{}: {}", tool_name, status),
                    });
                    true
                }),
            )
            .await?;

        let tx = self.dasein_tx.clone();
        event_bus
            .subscribe(
                EventType::MemoryStored,
                Box::new(move |event| {
                    let json = event.to_json();
                    let memory_type = json
                        .get("memory_type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    let content = json
                        .get("content")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let _ = tx.try_send(DaseinEvent::SystemEvent {
                        source: "memory".to_string(),
                        content: format!("[{}] {}", memory_type, content),
                    });
                    true
                }),
            )
            .await?;

        let tx = self.dasein_tx.clone();
        event_bus
            .subscribe(
                EventType::EvolutionTriggered,
                Box::new(move |event| {
                    let json = event.to_json();
                    let reason = json
                        .get("reason")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    let _ = tx.try_send(DaseinEvent::SystemEvent {
                        source: "evolution".to_string(),
                        content: format!("evolution triggered: {}", reason),
                    });
                    true
                }),
            )
            .await?;

        let tx = self.dasein_tx.clone();
        event_bus
            .subscribe(
                EventType::AgentStarted,
                Box::new(move |_event| {
                    let _ = tx.try_send(DaseinEvent::SystemEvent {
                        source: "session".to_string(),
                        content: "new session started".to_string(),
                    });
                    true
                }),
            )
            .await?;

        tracing::info!("DaseinEventBridge subscribed to EventBus");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bridge_creation() {
        let (tx, _rx) = mpsc::channel(16);
        let _bridge = DaseinEventBridge::new(tx);
        // Just verify construction succeeds
    }
}
