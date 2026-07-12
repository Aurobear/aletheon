//! DaseinEventBridge — bridges CommunicationBus topic events to DaseinModule.
//!
//! DaseinModule must perceive real system events to exist meaningfully.
//! This bridge subscribes to CommunicationBus topics (via SchemaId from EventType)
//! and translates system events into DaseinEvent messages on the DaseinModule's channel.

use fabric::events::types::EventType;
use fabric::ipc::envelope::Payload;
use fabric::ipc::envelope_v2::SchemaId;
use fabric::CommunicationBus;
use tokio::sync::mpsc;

use fabric::dasein::DaseinEvent;

/// Bridges CommunicationBus topic events to DaseinModule's internal event channel.
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

    /// Register topic subscriptions on the CommunicationBus to forward system
    /// events to the DaseinModule.
    ///
    /// Subscribes to:
    /// - `aletheon.event.tool_observation/v1` -- tool execution results update the involvement network
    /// - `aletheon.event.memory_stored/v1` -- memory events sediment into bewandtnis relations
    /// - `aletheon.event.evolution_triggered/v1` -- evolution events trigger negativity checks
    /// - `aletheon.event.agent_started/v1` -- session/lifecycle events update the temporal stream
    pub async fn subscribe(&self, communication_bus: &CommunicationBus) -> anyhow::Result<()> {
        // Helper: subscribe to a topic and spawn a background task that forwards
        // JSON payload data into a DaseinEvent via the provided mapping closure.
        fn spawn_topic_subscriber(
            bus: &CommunicationBus,
            event_type: EventType,
            tx: mpsc::Sender<DaseinEvent>,
            map: fn(serde_json::Value) -> DaseinEvent,
        ) {
            let schema = SchemaId::from_event_type(&event_type);
            let mut rx = bus.subscribe_topic(schema, Some(256));
            tokio::spawn(async move {
                while let Ok(envelope) = rx.recv().await {
                    let json = match &envelope.payload {
                        Payload::Json(v) => v.clone(),
                        _ => continue,
                    };
                    let event = map(json);
                    if tx.try_send(event).is_err() {
                        break; // channel closed
                    }
                }
            });
        }

        // ToolObservation → tool execution results
        {
            let tx = self.dasein_tx.clone();
            spawn_topic_subscriber(communication_bus, EventType::ToolObservation, tx, |json| {
                let tool_name = json
                    .get("tool_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                let status = json
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                DaseinEvent::SystemEvent {
                    source: "tool_execution".to_string(),
                    content: format!("{}: {}", tool_name, status),
                }
            });
        }

        // MemoryStored → memory events
        {
            let tx = self.dasein_tx.clone();
            spawn_topic_subscriber(communication_bus, EventType::MemoryStored, tx, |json| {
                let memory_type = json
                    .get("memory_type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                let content = json.get("content").and_then(|v| v.as_str()).unwrap_or("");
                DaseinEvent::SystemEvent {
                    source: "memory".to_string(),
                    content: format!("[{}] {}", memory_type, content),
                }
            });
        }

        // EvolutionTriggered → evolution events
        {
            let tx = self.dasein_tx.clone();
            spawn_topic_subscriber(
                communication_bus,
                EventType::EvolutionTriggered,
                tx,
                |json| {
                    let reason = json
                        .get("reason")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    DaseinEvent::SystemEvent {
                        source: "evolution".to_string(),
                        content: format!("evolution triggered: {}", reason),
                    }
                },
            );
        }

        // AgentStarted → session lifecycle
        {
            let tx = self.dasein_tx.clone();
            spawn_topic_subscriber(communication_bus, EventType::AgentStarted, tx, |_json| {
                DaseinEvent::SystemEvent {
                    source: "session".to_string(),
                    content: "new session started".to_string(),
                }
            });
        }

        tracing::info!("DaseinEventBridge subscribed to CommunicationBus topic events");
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
