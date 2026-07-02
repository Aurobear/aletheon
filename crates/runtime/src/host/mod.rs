//! Host abstraction (Tier 2b).
//!
//! A `RuntimeHost` is a deployment form of the runtime. `DaemonHost` is the
//! Unix-socket daemon. Additional hosts (systemd, container, CLI-one-shot) are
//! M-F, built on this trait.
//!
//! # Design
//!
//! - `init`: prepare resources (socket dirs, PID files, etc.)
//! - `serve`: run to completion (blocking on the host's event loop)
//! - `shutdown`: release resources
//! - Object-safe: `serve` takes `self: Box<Self>` for ownership transfer
//! - Phase 1 wraps `daemon::run` unchanged -- no protocol or behavior change

use anyhow::Result;
use std::path::PathBuf;

/// A deployment host for the runtime.
#[async_trait::async_trait(?Send)]
pub trait RuntimeHost {
    /// Prepare resources before serving. Called once at startup.
    async fn init(&mut self) -> Result<()>;

    /// Run the host's event loop to completion. Takes ownership.
    async fn serve(self: Box<Self>) -> Result<()>;

    /// Release resources. Called during graceful shutdown.
    async fn shutdown(&mut self) -> Result<()>;
}

/// The Unix-socket daemon host.
///
/// Wraps today's `daemon::run` unchanged. Socket-dir creation, PID-file
/// writing, and signal handling remain inside `daemon::run` for Phase 1
/// (behavior-identical). They migrate into `init()`/`shutdown()` when a
/// second host needs them (M-F).
pub struct DaemonHost {
    config_path: Option<PathBuf>,
    env_path: Option<PathBuf>,
    socket: PathBuf,
}

impl DaemonHost {
    pub fn new(
        config_path: Option<PathBuf>,
        env_path: Option<PathBuf>,
        socket: PathBuf,
    ) -> Self {
        Self {
            config_path,
            env_path,
            socket,
        }
    }
}

#[async_trait::async_trait(?Send)]
impl RuntimeHost for DaemonHost {
    async fn init(&mut self) -> Result<()> {
        // Socket dir creation etc. currently happens inside daemon::run.
        // Keep it there for Phase 1 so behavior is identical.
        // This hook exists for future hosts (M-F).
        Ok(())
    }

    async fn serve(self: Box<Self>) -> Result<()> {
        // Delegate to the existing, unchanged daemon entry point.
        // Path matches the existing import in bin/aletheond.rs:
        //   runtime::r#impl::daemon::run(...)
        crate::r#impl::daemon::run(
            self.config_path,
            self.env_path,
            self.socket,
        )
        .await
    }

    async fn shutdown(&mut self) -> Result<()> {
        // PID-file cleanup currently tracked inside daemon::run.
        // Placeholder for future hosts (M-F).
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    struct CountingHost {
        inited: Arc<AtomicUsize>,
        shut: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait(?Send)]
    impl RuntimeHost for CountingHost {
        async fn init(&mut self) -> Result<()> {
            self.inited.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        async fn serve(self: Box<Self>) -> Result<()> {
            Ok(())
        }
        async fn shutdown(&mut self) -> Result<()> {
            self.shut.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[tokio::test]
    async fn host_lifecycle_is_drivable() {
        let inited = Arc::new(AtomicUsize::new(0));
        let shut = Arc::new(AtomicUsize::new(0));
        let mut host = CountingHost {
            inited: inited.clone(),
            shut: shut.clone(),
        };
        host.init().await.unwrap();
        host.shutdown().await.unwrap();
        assert_eq!(inited.load(Ordering::SeqCst), 1);
        assert_eq!(shut.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn daemon_host_has_zero_init_shutdown_cost() {
        // init/shutdown are no-ops for DaemonHost in Phase 1.
        let mut host = DaemonHost::new(
            None,
            None,
            PathBuf::from("/tmp/test.sock"),
        );
        host.init().await.unwrap();
        host.shutdown().await.unwrap();
    }
}
