//! Bounded stream primitives for token/log/telemetry delivery.

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverflowPolicy {
    BlockProducer,
    DropOldest,
    DropNewest,
    FailStream,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamEndReason {
    Completed,
    Cancelled,
    Failed,
    Overflow,
    ReceiverClosed,
}

#[derive(Debug, Clone)]
pub struct StreamSpec {
    pub capacity: usize,
    pub overflow: OverflowPolicy,
    pub cancel: CancellationToken,
}

impl StreamSpec {
    pub fn llm_tokens(capacity: usize) -> Self {
        Self {
            capacity,
            overflow: OverflowPolicy::BlockProducer,
            cancel: CancellationToken::new(),
        }
    }

    pub fn telemetry(capacity: usize) -> Self {
        Self {
            capacity,
            overflow: OverflowPolicy::DropOldest,
            cancel: CancellationToken::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamSendError {
    Cancelled,
    Overflow,
    ReceiverClosed,
}

pub struct BoundedStream<T> {
    spec: StreamSpec,
    tx: mpsc::Sender<T>,
    rx: mpsc::Receiver<T>,
}

impl<T: Send + 'static> BoundedStream<T> {
    pub fn new(spec: StreamSpec) -> Self {
        let (tx, rx) = mpsc::channel(spec.capacity);
        Self { spec, tx, rx }
    }

    pub fn spec(&self) -> &StreamSpec {
        &self.spec
    }

    pub async fn send(&mut self, item: T) -> Result<(), StreamSendError> {
        if self.spec.cancel.is_cancelled() {
            return Err(StreamSendError::Cancelled);
        }
        match self.spec.overflow {
            OverflowPolicy::BlockProducer => self
                .tx
                .send(item)
                .await
                .map_err(|_| StreamSendError::ReceiverClosed),
            OverflowPolicy::DropNewest => match self.tx.try_send(item) {
                Ok(()) => Ok(()),
                Err(mpsc::error::TrySendError::Full(_)) => Ok(()),
                Err(mpsc::error::TrySendError::Closed(_)) => Err(StreamSendError::ReceiverClosed),
            },
            OverflowPolicy::FailStream => match self.tx.try_send(item) {
                Ok(()) => Ok(()),
                Err(mpsc::error::TrySendError::Full(_)) => Err(StreamSendError::Overflow),
                Err(mpsc::error::TrySendError::Closed(_)) => Err(StreamSendError::ReceiverClosed),
            },
            OverflowPolicy::DropOldest => match self.tx.try_send(item) {
                Ok(()) => Ok(()),
                Err(mpsc::error::TrySendError::Full(item)) => {
                    let _ = self.rx.try_recv();
                    self.tx.try_send(item).map_err(|e| match e {
                        mpsc::error::TrySendError::Full(_) => StreamSendError::Overflow,
                        mpsc::error::TrySendError::Closed(_) => StreamSendError::ReceiverClosed,
                    })
                }
                Err(mpsc::error::TrySendError::Closed(_)) => Err(StreamSendError::ReceiverClosed),
            },
        }
    }

    pub async fn recv(&mut self) -> Option<T> {
        self.rx.recv().await
    }

    pub fn try_recv(&mut self) -> Option<T> {
        self.rx.try_recv().ok()
    }
}
