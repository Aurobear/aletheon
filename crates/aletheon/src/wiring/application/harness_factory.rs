//! Retired General factory entry point — one-way compatibility re-export.
//!
//! The concrete `HarnessCognitiveSessionFactory` implementation and
//! `selected_harness_kind` now live in `cognit::harness::factory`; the
//! `CognitiveRuntimeConfig`-based `production_cognitive_session_factory` and
//! `harness_config_from_runtime` adapters live in the binary-owned
//! `aletheon::wiring::composition::harness_factory`. This module only re-exports
//! the public surface so existing callers and tests keep compiling while the
//! cutover completes. No new implementation is added here.

pub use cognit::harness::{
    selected_harness_kind, CognitiveSessionFactory, ExecutionTargetRoutingError,
    HarnessCognitiveSessionFactory, LinearCognitiveSessionFactory, RobotSessionCapability,
    TargetRoutedCognitiveSessionFactory,
};

#[cfg(test)]
mod tests {
    use super::*;
    use ::contracts::SessionRecord;
    use async_trait::async_trait;
    use cognit::harness::HarnessConfig;
    use runtime::turn_policy::TurnPolicy;
    use tokio_util::sync::CancellationToken;

    /// Source-shape regression: the General production factory must keep
    /// constructing the Harness core, never the legacy `ReActLoop` or a
    /// `LinearCognitiveSession` directly. This module is now a pure re-export
    /// seam; the concrete construction lives in `cognit::harness::factory` and
    /// `aletheon::wiring::composition::harness_factory`.
    #[test]
    fn production_factory_never_constructs_legacy_loop() {
        let production = include_str!("harness_factory.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap_or_default();
        for forbidden in ["ReActLoop::", "LinearCognitiveSession::new"] {
            assert!(
                !production.contains(forbidden),
                "harness_factory.rs production source must not contain {forbidden}"
            );
        }
        assert!(
            production.contains("pub use cognit::harness"),
            "harness_factory.rs must re-export the cognit-owned factory surface"
        );
    }

    struct MarkerFactory(&'static str);

    #[async_trait]
    impl CognitiveSessionFactory for MarkerFactory {
        async fn create(
            &self,
            _session: &SessionRecord,
            _policy: &TurnPolicy,
            _cancellation: CancellationToken,
        ) -> anyhow::Result<Box<dyn cognit::harness::CognitiveSession>> {
            anyhow::bail!(self.0)
        }
    }

    fn session_record() -> SessionRecord {
        SessionRecord {
            schema_version: ::contracts::SESSION_SCHEMA_VERSION,
            id: ::contracts::SessionId("target-routing".into()),
            parent: None,
            created_at_ms: 0,
            status: ::contracts::SessionStatus::Active,
        }
    }

    #[tokio::test]
    async fn target_router_defaults_to_general_and_uses_robot_only_when_explicit() {
        let router = TargetRoutedCognitiveSessionFactory::new(
            std::sync::Arc::new(MarkerFactory("general factory selected")),
            Some(RobotSessionCapability::new(
                std::sync::Arc::new(MarkerFactory("robot factory selected")),
                ::contracts::types::embodiment::DeviceId("robot-1".into()),
                ::contracts::types::embodiment::ExecutionEnvironment::Simulation,
            )),
        );
        let session = session_record();

        let general = router
            .create_configured_for_target(
                &session,
                &TurnPolicy::daemon(),
                &::contracts::ExecutionTargetSelection::default(),
                HarnessConfig::default(),
                CancellationToken::new(),
                None,
            )
            .await
            .err()
            .expect("marker factory must fail");
        assert_eq!(general.to_string(), "general factory selected");

        let robot = ::contracts::ExecutionTargetSelection::robot(
            "robot-1",
            ::contracts::types::embodiment::ExecutionEnvironment::Simulation,
            ::contracts::ExecutionTargetSource::UserCommand,
        )
        .unwrap();
        let selected = router
            .create_configured_for_target(
                &session,
                &TurnPolicy::daemon(),
                &robot,
                HarnessConfig::default(),
                CancellationToken::new(),
                None,
            )
            .await
            .err()
            .expect("marker factory must fail");
        assert_eq!(selected.to_string(), "robot factory selected");
    }

    #[tokio::test]
    async fn target_router_rejects_unconfigured_or_mismatched_robot_binding() {
        let session = session_record();
        let target = ::contracts::ExecutionTargetSelection::robot(
            "other-robot",
            ::contracts::types::embodiment::ExecutionEnvironment::Simulation,
            ::contracts::ExecutionTargetSource::TrustedClient,
        )
        .unwrap();
        let unavailable = TargetRoutedCognitiveSessionFactory::new(
            std::sync::Arc::new(MarkerFactory("general factory selected")),
            None,
        )
        .validate_target(&target)
        .unwrap_err();
        assert!(matches!(
            unavailable,
            ExecutionTargetRoutingError::Unavailable(_)
        ));

        let router = TargetRoutedCognitiveSessionFactory::new(
            std::sync::Arc::new(MarkerFactory("general factory selected")),
            Some(RobotSessionCapability::new(
                std::sync::Arc::new(MarkerFactory("robot factory selected")),
                ::contracts::types::embodiment::DeviceId("robot-1".into()),
                ::contracts::types::embodiment::ExecutionEnvironment::Simulation,
            )),
        );
        let error = router
            .create_configured_for_target(
                &session,
                &TurnPolicy::daemon(),
                &target,
                HarnessConfig::default(),
                CancellationToken::new(),
                None,
            )
            .await
            .err()
            .expect("mismatched device must fail");
        assert!(error.to_string().contains("device 'other-robot'"));
    }
}
