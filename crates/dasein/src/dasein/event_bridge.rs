//! DaseinEventBridge — bridges canonical events to DaseinModule.
//!
//! DaseinModule must perceive real system events to exist meaningfully.
//! This bridge subscribes to canonical schemas
//! and translates system events into DaseinEvent messages on the DaseinModule's channel.

use fabric::ipc::envelope_v2::SchemaId;
use fabric::CanonicalEventBus;
use tokio::sync::mpsc;

use fabric::dasein::DaseinEvent;

/// Bridges canonical events to DaseinModule's internal event channel.
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

    /// Register schema subscriptions on the canonical event bus to forward system
    /// events to the DaseinModule.
    ///
    /// Subscribes to:
    /// - `aletheon.event.tool_observation/v1` -- tool execution results update the involvement network
    /// - `aletheon.event.memory_stored/v1` -- memory events sediment into bewandtnis relations
    /// - `aletheon.event.evolution_triggered/v1` -- evolution events trigger negativity checks
    /// - `aletheon.event.agent_started/v1` -- session/lifecycle events update the temporal stream
    pub async fn subscribe(&self, event_bus: &CanonicalEventBus) -> anyhow::Result<()> {
        // Helper: subscribe to a topic and spawn a background task that forwards
        // JSON payload data into a DaseinEvent via the provided mapping closure.
        fn spawn_topic_subscriber(
            bus: &CanonicalEventBus,
            schema: SchemaId,
            tx: mpsc::Sender<DaseinEvent>,
            map: fn(serde_json::Value) -> DaseinEvent,
        ) {
            let mut rx = bus.subscribe_channel(schema);
            tokio::spawn(async move {
                while let Ok(envelope) = rx.recv().await {
                    let json = envelope.payload;
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
            spawn_topic_subscriber(
                event_bus,
                SchemaId(SchemaId::EVENT_TOOL_OBSERVATION_V1.into()),
                tx,
                |json| {
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
                        content: format!("{tool_name}: {status}"),
                    }
                },
            );
        }

        // MemoryStored → memory events
        {
            let tx = self.dasein_tx.clone();
            spawn_topic_subscriber(
                event_bus,
                SchemaId(SchemaId::EVENT_MEMORY_STORED_V1.into()),
                tx,
                |json| {
                    let memory_type = json
                        .get("memory_type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    let content = json.get("content").and_then(|v| v.as_str()).unwrap_or("");
                    DaseinEvent::SystemEvent {
                        source: "memory".to_string(),
                        content: format!("[{memory_type}] {content}"),
                    }
                },
            );
        }

        // EvolutionTriggered → evolution events
        {
            let tx = self.dasein_tx.clone();
            spawn_topic_subscriber(
                event_bus,
                SchemaId(SchemaId::EVENT_EVOLUTION_TRIGGERED_V1.into()),
                tx,
                |json| {
                    let reason = json
                        .get("reason")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    DaseinEvent::SystemEvent {
                        source: "evolution".to_string(),
                        content: format!("evolution triggered: {reason}"),
                    }
                },
            );
        }

        // AgentStarted → session lifecycle
        {
            let tx = self.dasein_tx.clone();
            spawn_topic_subscriber(
                event_bus,
                SchemaId(SchemaId::EVENT_AGENT_STARTED_V1.into()),
                tx,
                |_json| DaseinEvent::SystemEvent {
                    source: "session".to_string(),
                    content: "new session started".to_string(),
                },
            );
        }

        tracing::info!("DaseinEventBridge subscribed to canonical event schemas");
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
