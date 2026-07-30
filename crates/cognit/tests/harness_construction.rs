use std::pin::Pin;
use std::sync::Arc;

use cognit::harness::{
    build_harness, CompactorTrait, HarnessBuildError, HarnessConfig, HarnessKind,
};
use fabric::message::Message;
use fabric::LlmProvider;
use kernel::chronos::TestClock;

struct NoopCompactor;

impl CompactorTrait for NoopCompactor {
    fn maybe_compact<'a>(
        &'a mut self,
        _messages: &'a mut Vec<Message>,
        _llm: &'a dyn LlmProvider,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<bool>> + Send + 'a>> {
        Box::pin(async { Ok(false) })
    }

    fn force_compact<'a>(
        &'a mut self,
        _messages: &'a mut Vec<Message>,
        _llm: &'a dyn LlmProvider,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<bool>> + Send + 'a>> {
        Box::pin(async { Ok(false) })
    }
}

#[test]
fn linear_harness_constructs_through_generic_factory() {
    let result = build_harness(
        HarnessKind::Linear,
        HarnessConfig::default(),
        Box::new(NoopCompactor),
        Arc::new(TestClock::default()),
    );
    assert!(result.is_ok());
}

#[test]
fn robot_harness_requires_executive_ports_without_panicking() {
    let result = build_harness(
        HarnessKind::Robot,
        HarnessConfig::default(),
        Box::new(NoopCompactor),
        Arc::new(TestClock::default()),
    );
    assert!(matches!(
        result,
        Err(HarnessBuildError::RequiresExecutivePorts {
            kind: HarnessKind::Robot
        })
    ));
}
