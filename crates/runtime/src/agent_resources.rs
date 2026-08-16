//! Runtime-owned background resource registration lifecycle (RA-05).
//!
//! The registration is a small lifecycle fence used by Agent resources.  It
//! intentionally contains no Executive, Kernel, or concrete adapter types;
//! host code only supplies a cancellation-aware producer and observes the
//! bounded stopped state.

use ::contracts::{AgentControlError, AgentControlErrorKind};
use std::future::Future;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

const REGISTRATION_UNBOUND: u8 = 0;
const REGISTRATION_RUNNING: u8 = 1;
const REGISTRATION_STOPPED: u8 = 2;

#[derive(Clone, Debug)]
pub struct BackgroundResourceRegistration {
    token: CancellationToken,
    state: Arc<AtomicU8>,
    stopped: Arc<tokio::sync::Notify>,
}

impl BackgroundResourceRegistration {
    pub fn new(token: CancellationToken) -> Self {
        Self {
            token,
            state: Arc::new(AtomicU8::new(REGISTRATION_UNBOUND)),
            stopped: Arc::new(tokio::sync::Notify::new()),
        }
    }

    pub fn cancellation(&self) -> CancellationToken {
        self.token.clone()
    }

    pub fn bind<F, Fut>(&self, producer: F) -> Result<(), AgentControlError>
    where
        F: FnOnce(CancellationToken) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.state
            .compare_exchange(
                REGISTRATION_UNBOUND,
                REGISTRATION_RUNNING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .map_err(|_| invalid("background producer is already registered"))?;
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| {
            self.state.store(REGISTRATION_UNBOUND, Ordering::Release);
            invalid("background producer registration requires a Tokio runtime")
        })?;
        let token = self.token.clone();
        let producer_token = token.clone();
        let state = self.state.clone();
        let stopped = self.stopped.clone();
        runtime.spawn(async move {
            let producer = producer(producer_token);
            tokio::pin!(producer);
            tokio::select! {
                _ = &mut producer => {}
                _ = token.cancelled() => {
                    let _ = tokio::time::timeout(
                        std::time::Duration::from_secs(5),
                        &mut producer,
                    ).await;
                }
            }
            state.store(REGISTRATION_STOPPED, Ordering::Release);
            stopped.notify_waiters();
        });
        Ok(())
    }

    pub async fn cancel_and_wait(&self) {
        self.token.cancel();
        self.wait_stopped().await;
    }

    pub async fn wait_stopped(&self) {
        loop {
            let notified = self.stopped.notified();
            if self.state.load(Ordering::Acquire) != REGISTRATION_RUNNING {
                return;
            }
            notified.await;
        }
    }

    pub fn is_stopped(&self) -> bool {
        self.state.load(Ordering::Acquire) == REGISTRATION_STOPPED
    }
}

fn invalid(message: &str) -> AgentControlError {
    AgentControlError {
        kind: AgentControlErrorKind::Runtime,
        message: message.into(),
    }
}
