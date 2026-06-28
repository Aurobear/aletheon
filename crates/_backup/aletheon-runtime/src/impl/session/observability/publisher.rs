use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tracing::{debug, warn};

/// Observable session events for the observability stack.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionEvent {
    /// An LLM inference call was made.
    LlmCall {
        model: String,
        input_tokens: u64,
        output_tokens: u64,
        latency_ms: u64,
    },
    /// A tool call was initiated or completed.
    ToolCall {
        tool_call_id: String,
        tool_name: String,
        phase: ToolCallPhase,
        elapsed_ms: Option<u64>,
        is_error: bool,
    },
    /// A lifecycle hook executed.
    HookExecution {
        hook_name: String,
        hook_type: String,
        elapsed_ms: u64,
        success: bool,
    },
    /// Context was compacted.
    Compaction {
        before_messages: usize,
        after_messages: usize,
    },
    /// An error occurred.
    Error {
        component: String,
        message: String,
    },
}

/// Phase of a tool call lifecycle.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallPhase {
    Started,
    Completed,
}

/// A subscriber handle identified by name.
type SubscriberMap = HashMap<String, mpsc::Sender<SessionEvent>>;

/// Multi-subscriber event publisher using mpsc channels.
pub struct EventPublisher {
    subscribers: SubscriberMap,
}

impl EventPublisher {
    /// Create a new empty publisher.
    pub fn new() -> Self {
        Self {
            subscribers: HashMap::new(),
        }
    }

    /// Add a live subscriber with the given name and channel capacity.
    /// Returns the receiving end of the channel.
    pub fn add_live_subscriber(&mut self, name: impl Into<String>, capacity: usize) -> mpsc::Receiver<SessionEvent> {
        let (tx, rx) = mpsc::channel(capacity);
        let name = name.into();
        debug!(subscriber = %name, capacity, "Added live subscriber");
        self.subscribers.insert(name, tx);
        rx
    }

    /// Publish an event to all live subscribers.
    /// Removes subscribers whose channels have been closed.
    pub async fn publish(&mut self, event: SessionEvent) {
        let mut closed = Vec::new();

        for (name, tx) in &self.subscribers {
            if tx.send(event.clone()).await.is_err() {
                warn!(subscriber = %name, "Subscriber channel closed, marking for removal");
                closed.push(name.clone());
            }
        }

        for name in closed {
            self.subscribers.remove(&name);
        }
    }

    /// Remove all disconnected subscribers.
    pub fn cleanup_subscribers(&mut self) {
        let before = self.subscribers.len();
        self.subscribers.retain(|_, tx| !tx.is_closed());
        let removed = before - self.subscribers.len();
        if removed > 0 {
            debug!(removed, remaining = self.subscribers.len(), "Cleaned up subscribers");
        }
    }

    /// Number of active subscribers.
    pub fn subscriber_count(&self) -> usize {
        self.subscribers.len()
    }
}

impl Default for EventPublisher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_publish_to_single_subscriber() {
        let mut publisher = EventPublisher::new();
        let mut rx = publisher.add_live_subscriber("test", 16);

        let event = SessionEvent::Error {
            component: "test".into(),
            message: "hello".into(),
        };
        publisher.publish(event.clone()).await;

        let received = rx.recv().await.unwrap();
        match received {
            SessionEvent::Error { component, message } => {
                assert_eq!(component, "test");
                assert_eq!(message, "hello");
            }
            _ => panic!("Unexpected event variant"),
        }
    }

    #[tokio::test]
    async fn test_publish_to_multiple_subscribers() {
        let mut publisher = EventPublisher::new();
        let mut rx1 = publisher.add_live_subscriber("sub1", 16);
        let mut rx2 = publisher.add_live_subscriber("sub2", 16);

        let event = SessionEvent::Compaction {
            before_messages: 100,
            after_messages: 20,
        };
        publisher.publish(event).await;

        let r1 = rx1.recv().await.unwrap();
        let r2 = rx2.recv().await.unwrap();
        assert!(matches!(r1, SessionEvent::Compaction { .. }));
        assert!(matches!(r2, SessionEvent::Compaction { .. }));
        assert_eq!(publisher.subscriber_count(), 2);
    }

    #[tokio::test]
    async fn test_cleanup_removes_closed_subscribers() {
        let mut publisher = EventPublisher::new();
        let _rx1 = publisher.add_live_subscriber("keep", 16);
        let rx2 = publisher.add_live_subscriber("drop", 16);

        drop(rx2);
        // Send to trigger detection of closed channel
        let event = SessionEvent::Error {
            component: "c".into(),
            message: "m".into(),
        };
        publisher.publish(event).await;

        // The closed subscriber should have been removed after publish
        assert!(publisher.subscriber_count() <= 2);
        // Explicit cleanup
        publisher.cleanup_subscribers();
        assert_eq!(publisher.subscriber_count(), 1);
    }

    #[tokio::test]
    async fn test_no_subscribers_does_not_panic() {
        let mut publisher = EventPublisher::new();
        let event = SessionEvent::LlmCall {
            model: "gpt-4".into(),
            input_tokens: 100,
            output_tokens: 50,
            latency_ms: 500,
        };
        publisher.publish(event).await;
        assert_eq!(publisher.subscriber_count(), 0);
    }
}
