//! Deterministic builder for fully wired, isolated test Aletheon instances.
//!
//! Constructs a `TurnCoordinator` with in-memory stores and an injectable
//! `TestClock` for deterministic time. Tests provide their own runner closures
//! (using `MockLlmProvider` and `MockSandbox`) to execute turns.
//!
//! # Usage
//!
//! ```ignore
//! let test = TestAletheonBuilder::new().build().await;
//! let process = test.kernel.spawn_process(SpawnSpec::default()).await.unwrap();
//! let result = test.coordinator.submit_with(
//!     request("session-1", process.id),
//!     &TurnPolicy::daemon(),
//!     |request, cancel| async { ... use mock_llm and mock_sandbox ... },
//! ).await;
//! // Assert on items stored via test.store
//! ```

use std::sync::Arc;

use aletheon_kernel::chronos::TestClock;
use aletheon_kernel::KernelRuntime;
use executive::r#impl::events::SqliteEventSpine;
use executive::r#impl::session::canonical_store::CanonicalSessionStore;
use executive::service::turn_coordinator::TurnCoordinator;
use fabric::{Clock, SessionAppendStore};

/// A fully constructed test Aletheon, ready for test execution.
pub struct TestAletheon {
    pub kernel: Arc<KernelRuntime>,
    pub clock: Arc<TestClock>,
    pub store: Arc<dyn SessionAppendStore>,
    pub event_spine: Arc<SqliteEventSpine>,
    pub coordinator: TurnCoordinator,
}

/// Builder for a fully wired, deterministic test Aletheon instance.
///
/// Defaults: in-memory SQLite, TestClock(wall=0, mono=0).
pub struct TestAletheonBuilder {
    clock: Option<Arc<TestClock>>,
}

impl TestAletheonBuilder {
    /// Create a new builder with all-memory defaults and zeroed TestClock.
    pub fn new() -> Self {
        Self { clock: None }
    }

    /// Use a specific TestClock instead of the default (wall=0, mono=0).
    pub fn with_clock(mut self, clock: Arc<TestClock>) -> Self {
        self.clock = Some(clock);
        self
    }

    /// Build the fully wired test instance.
    ///
    /// Constructs:
    /// - TestClock (or the injected one)
    /// - KernelRuntime with the TestClock
    /// - In-memory CanonicalSessionStore
    /// - In-memory SqliteEventSpine
    /// - TurnCoordinator (with event-sourced store)
    pub async fn build(self) -> TestAletheon {
        let clock = self.clock.unwrap_or_else(|| Arc::new(TestClock::default()));
        let kernel = Arc::new(KernelRuntime::with_clock(clock.clone() as Arc<dyn Clock>));
        let store: Arc<dyn SessionAppendStore> =
            Arc::new(CanonicalSessionStore::open(":memory:").expect("in-memory session store"));
        let event_spine = Arc::new(
            SqliteEventSpine::open(":memory:").expect("in-memory event spine"),
        );
        let coordinator =
            TurnCoordinator::with_event_spine(kernel.clone(), store.clone(), event_spine.clone());

        TestAletheon {
            kernel,
            clock,
            store,
            event_spine,
            coordinator,
        }
    }
}

impl Default for TestAletheonBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use executive::service::turn_policy::TurnPolicy;
    use fabric::{
        ItemPayload, SpawnSpec, TurnMetrics, TurnResult, TurnStop,
    };

    fn request(session: &str, process_id: fabric::ProcessId) -> fabric::TurnRequest {
        fabric::TurnRequest {
            operation_id: fabric::OperationId::default(),
            process_id,
            context: crate::turn_request_support::context(session, std::env::temp_dir()),
            input: "hello".into(),
            model_policy: None,
            deadline: None,
        }
    }

    #[tokio::test]
    async fn build_creates_fully_wired_instance() {
        let test = TestAletheonBuilder::new().build().await;

        // Clock is accessible
        assert_eq!(test.clock.mono_now().0, 0);

        // Store is functional
        let store_snapshot = test.store.clone();
        assert!(store_snapshot.load_items(&fabric::SessionId("nonexistent".into()), None).await.is_ok());

        // Coordinator is functional
        assert_eq!(test.coordinator.active_turn_count().await, 0);
    }

    #[tokio::test]
    async fn build_with_custom_clock_preserves_time() {
        let clock = Arc::new(TestClock::new(1000, 500));
        let test = TestAletheonBuilder::new()
            .with_clock(clock.clone())
            .build()
            .await;

        assert_eq!(test.clock.wall_now().0, 1000);
        assert_eq!(test.clock.mono_now().0, 500);
    }

    #[tokio::test]
    async fn submit_simple_turn_writes_items_and_settles_operation() {
        let test = TestAletheonBuilder::new().build().await;
        let process = test
            .kernel
            .spawn_process(SpawnSpec::default())
            .await
            .unwrap();

        let result = test
            .coordinator
            .submit_with(
                request("builder-test", process.id),
                &TurnPolicy::daemon(),
                |request, _cancel| {
                    let output = request.input.clone();
                    async move {
                        Ok(executive::service::turn_coordinator::TurnExecution {
                            result: TurnResult {
                                output: format!("answer: {output}"),
                                stop: TurnStop::Completed,
                                metrics: TurnMetrics {
                                    completed_normally: true,
                                    ..Default::default()
                                },
                            },
                            items: vec![],
                            projection: None,
                            context_projection: None,
                        })
                    }
                },
            )
            .await
            .unwrap();

        assert_eq!(result.output, "answer: hello");
        assert_eq!(result.stop, TurnStop::Completed);

        // Items are persisted
        let items = test
            .store
            .load_items(&fabric::SessionId("builder-test".into()), None)
            .await
            .unwrap();
        assert_eq!(items.len(), 2);
        assert!(matches!(items[0].payload, ItemPayload::UserMessage { .. }));
        assert!(matches!(
            items[1].payload,
            ItemPayload::AssistantMessage { .. }
        ));
    }
}
