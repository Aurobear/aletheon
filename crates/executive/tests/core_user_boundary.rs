use std::sync::Arc;

use executive::core::{RegistryInferencePort, SystemCoreRuntime};
use executive::service::inference_port::{CoreInferenceRequest, InferenceError, InferencePort};
use executive::user_runtime::{UserRuntime, UserRuntimeConfig};
use fabric::{LlmResponse, LlmStream, StopReason, Usage};
use futures::stream;

#[derive(Default)]
struct FakeInferencePort;

#[async_trait::async_trait]
impl InferencePort for FakeInferencePort {
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
        usage: Usage::default(),
        cache_hit_tokens: 0,
        cache_miss_tokens: 0,
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
    let port = RegistryInferencePort::fixture_with_alias("fast", "openai/gpt-test");
    assert_eq!(port.resolve_model("fast").unwrap().model, "gpt-test");
    assert!(port
        .resolve_model("missing-provider/model")
        .unwrap_err()
        .to_string()
        .contains("Provider 'missing-provider' not found"));
}

#[test]
fn system_core_surface_exposes_no_user_execution_authority() {
    fn accepts_core(_: &SystemCoreRuntime) {}
    accepts_core(&SystemCoreRuntime::fixture());
    let core = include_str!("../src/core/system_core_runtime.rs");
    for forbidden in ["RequestHandler", "ToolRegistry", "Sandbox"] {
        assert!(!core.contains(forbidden), "core contains {forbidden}");
    }
    let user = include_str!("../src/user_runtime/mod.rs");
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
    let error = SystemCoreRuntime::bootstrap(Some(&config), directory.path().join("core.sock"))
        .await
        .err()
        .expect("user integration config must be rejected");
    assert!(error
        .to_string()
        .contains("user-scoped integration credentials"));
}
