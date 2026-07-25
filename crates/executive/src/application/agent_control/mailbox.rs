//! Kernel-mailbox adapter for one live Agent runtime.

use std::sync::Arc;

use fabric::ipc::envelope_v2::{EnvelopeV2, SchemaId};
use fabric::ipc::mailbox::Mailbox;
use fabric::{AgentControlError, AgentControlErrorKind, AgentMessageKind, AgentMessagePayload};
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone)]
pub struct AgentRuntimeInbox {
    receiver: Arc<Mutex<mpsc::Receiver<AgentMessagePayload>>>,
}

impl AgentRuntimeInbox {
    pub fn empty() -> Self {
        let (_sender, receiver) = mpsc::channel(1);
        Self {
            receiver: Arc::new(Mutex::new(receiver)),
        }
    }

    pub fn bounded_channel(
        capacity: usize,
    ) -> Result<(mpsc::Sender<AgentMessagePayload>, Self), AgentControlError> {
        if capacity == 0 {
            return Err(AgentControlError::invalid(
                "Agent runtime inbox capacity must be nonzero",
            ));
        }
        let (sender, receiver) = mpsc::channel(capacity);
        Ok((
            sender,
            Self {
                receiver: Arc::new(Mutex::new(receiver)),
            },
        ))
    }

    pub async fn recv(&self) -> Option<AgentMessagePayload> {
        self.receiver.lock().await.recv().await
    }

    pub async fn try_recv(&self) -> Option<AgentMessagePayload> {
        self.receiver.lock().await.try_recv().ok()
    }
}

pub struct AgentMailboxBridge {
    mailbox: Arc<dyn Mailbox>,
    sender: mpsc::Sender<AgentMessagePayload>,
    cancellation: CancellationToken,
}

impl AgentMailboxBridge {
    pub fn bounded(
        mailbox: Arc<dyn Mailbox>,
        capacity: usize,
        cancellation: CancellationToken,
    ) -> Result<(Self, AgentRuntimeInbox), AgentControlError> {
        if capacity == 0 {
            return Err(AgentControlError::invalid(
                "Agent runtime inbox capacity must be nonzero",
            ));
        }
        let (sender, receiver) = mpsc::channel(capacity);
        Ok((
            Self {
                mailbox,
                sender,
                cancellation,
            },
            AgentRuntimeInbox {
                receiver: Arc::new(Mutex::new(receiver)),
            },
        ))
    }

    pub async fn run(self) -> Result<(), AgentControlError> {
        loop {
            let envelope = tokio::select! {
                _ = self.cancellation.cancelled() => return Ok(()),
                envelope = self.mailbox.recv() => match envelope {
                    Some(envelope) => envelope,
                    None => return Ok(()),
                },
            };
            let payload = decode(envelope)?;
            if payload.kind == AgentMessageKind::Signal {
                self.cancellation.cancel();
                return Ok(());
            }
            tokio::select! {
                _ = self.cancellation.cancelled() => return Ok(()),
                result = self.sender.send(payload) => {
                    if result.is_err() {
                        return Ok(());
                    }
                }
            }
        }
    }
}

fn decode(envelope: EnvelopeV2) -> Result<AgentMessagePayload, AgentControlError> {
    if envelope.schema.0 != SchemaId::AGENT_CONTROL_MESSAGE_V1 {
        return Err(AgentControlError {
            kind: AgentControlErrorKind::InvalidRequest,
            message: format!("unsupported Agent mailbox schema: {}", envelope.schema.0),
        });
    }
    let payload = envelope
        .payload
        .get("payload")
        .cloned()
        .ok_or_else(|| AgentControlError::invalid("Agent mailbox payload is missing"))?;
    let payload: AgentMessagePayload = serde_json::from_value(payload)
        .map_err(|_| AgentControlError::invalid("Agent mailbox payload is malformed"))?;
    payload.validate()?;
    Ok(payload)
}
