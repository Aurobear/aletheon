use std::sync::Arc;

use ::contracts::{InferenceUsage, LlmResponse, LlmStream, StopReason};
use aletheon::wiring::core_runtime::MachineInferenceRuntime;
use aletheon::wiring::user_runtime::{UserRuntime, UserRuntimeConfig};
use cognit::ports::inference::{
    CoreInferenceRequest, InferenceError, InferencePort, ModelCapabilities,
};
use futures::stream;

#[derive(Default)]
struct FakeInferencePort;

#[async_trait::async_trait]
impl InferencePort for FakeInferencePort {
    async fn capabilities(&self, model_spec: &str) -> Result<ModelCapabilities, InferenceError> {
        Ok(ModelCapabilities {
            provider_id: None,
            transport: None,
            cache_reporting: None,
            model_spec: if model_spec.is_empty() {
                "fixture/fixture-model".into()
            } else {
                model_spec.to_owned()
            },
            display_name: "fixture-model".into(),
            max_context_tokens: 1_000_000,
        })
    }

    async fn complete(
        &self,
        _request: CoreInferenceRequest,
    ) -> Result<LlmResponse, InferenceError> {
        Ok(response())
    }

    async fn stream(&self, _request: CoreInferenceRequest) -> Result<LlmStream, InferenceError> {
        Ok(Box::pin(stream::empty()))
    }
}

fn response() -> LlmResponse {
    LlmResponse {
        content: Vec::new(),
        stop_reason: StopReason::EndTurn,
        usage: InferenceUsage::default(),
    }
}

#[tokio::test]
async fn user_runtime_builds_from_inference_port_without_provider_registry() {
    let runtime = UserRuntime::bootstrap(UserRuntimeConfig::fixture(), Arc::new(FakeInferencePort))
        .await
        .unwrap();
    runtime.health().await.unwrap();
}

#[tokio::test]
async fn two_user_runtime_configs_never_share_state_paths() {
    let alice_root = tempfile::tempdir().unwrap();
    let bob_root = tempfile::tempdir().unwrap();
    let alice = UserRuntime::bootstrap(
        UserRuntimeConfig::fixture_at(alice_root.path()),
        Arc::new(FakeInferencePort),
    )
    .await
    .unwrap();
    let bob = UserRuntime::bootstrap(
        UserRuntimeConfig::fixture_at(bob_root.path()),
        Arc::new(FakeInferencePort),
    )
    .await
    .unwrap();
    assert_ne!(alice.state_paths(), bob.state_paths());
    assert!(alice
        .state_paths()
        .iter()
        .all(|path| path.starts_with(alice_root.path())));
    assert!(bob
        .state_paths()
        .iter()
        .all(|path| path.starts_with(bob_root.path())));
}

#[test]
fn core_registry_resolves_requested_models_and_rejects_unknown_providers() {
    let source = include_str!("../src/wiring/core_runtime.rs");
    assert!(source.contains("ProviderRegistry::from_config"));
    assert!(source.contains("resolve_and_create"));
}

#[test]
fn system_core_surface_exposes_no_user_execution_authority() {
    fn accepts_core(_: &MachineInferenceRuntime) {}
    let _ = accepts_core;
    let core = include_str!("../src/wiring/core_runtime.rs");
    for forbidden in ["RequestHandler", "ToolRegistry", "Sandbox"] {
        assert!(!core.contains(forbidden), "core contains {forbidden}");
    }
    let user = include_str!("../src/wiring/user_runtime.rs");
    for forbidden in ["ProviderRegistry", "credential loading"] {
        assert!(
            !user.contains(forbidden),
            "user runtime contains {forbidden}"
        );
    }
    assert!(!user.contains("api_key"));
    assert!(!user.contains("api_url"));
}

#[tokio::test]
async fn system_core_rejects_user_scoped_integration_configuration() {
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("core.toml");
    std::fs::write(&config, "[telegram]\nenabled = true\n").unwrap();
    let error =
        MachineInferenceRuntime::bootstrap(Some(&config), directory.path().join("core.sock"))
            .await
            .err()
            .expect("user integration config must be rejected");
    assert!(error
        .to_string()
        .contains("user-scoped integration credentials"));
}
