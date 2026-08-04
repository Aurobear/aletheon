use super::*;
use kernel::chronos::SystemClock;
use tokio_util::sync::CancellationToken;

#[cfg(test)]
mod goal_runtime_tests {
    use super::*;
    use crate::composition::config::AppConfig;
    use cognit::config::{GoalRuntimeConfig, ProviderConfig, RoleRuntimeConfig, Transport};

    struct NoopCapability;

    #[derive(Default)]
    struct NoopInference;

    #[async_trait::async_trait]
    impl InferencePort for NoopInference {
        async fn capabilities(
            &self,
            model_spec: &str,
        ) -> Result<
            crate::application::inference_port::ModelCapabilities,
            crate::application::inference_port::InferenceError,
        > {
            Ok(crate::application::inference_port::ModelCapabilities {
                model_spec: model_spec.into(),
                display_name: model_spec.into(),
                max_context_tokens: 128_000,
            })
        }

        async fn complete(
            &self,
            _request: crate::application::inference_port::CoreInferenceRequest,
        ) -> Result<fabric::LlmResponse, crate::application::inference_port::InferenceError>
        {
            Err(anyhow::anyhow!("unused test inference").into())
        }

        async fn stream(
            &self,
            _request: crate::application::inference_port::CoreInferenceRequest,
        ) -> Result<fabric::LlmStream, crate::application::inference_port::InferenceError> {
            Err(anyhow::anyhow!("unused test inference").into())
        }
    }

    #[async_trait::async_trait]
    impl CapabilityService for NoopCapability {
        async fn invoke(
            &self,
            _context: Option<crate::application::CapabilityExecutionContext>,
            call: fabric::CapabilityCall,
            _cancel: CancellationToken,
        ) -> fabric::CapabilityResult {
            fabric::CapabilityResult {
                call_id: call.call_id,
                output: "unused".into(),
                is_error: true,
                usage: fabric::UsageReport::default(),
                audit_id: None,
                patch_delta: None,
                served_from_cache: false,
            }
        }
    }

    fn provider(name: &str) -> ProviderConfig {
        ProviderConfig {
            name: name.into(),
            base_url: "http://127.0.0.1:1".into(),
            api_key: String::new(),
            transport: Transport::Openai,
            models: vec!["model".into()],
            max_context_length: None,
            pricing: None,
            backpressure: Default::default(),
            cache: Default::default(),
        }
    }

    fn route(runtime_id: &str, model_alias: &str) -> RoleRuntimeConfig {
        RoleRuntimeConfig {
            runtime_id: runtime_id.into(),
            model_alias: model_alias.into(),
            max_steps: 2,
            max_persisted_bytes: 1024,
            allowed_tools: vec![],
        }
    }

    async fn register(
        config: GoalRuntimeConfig,
        app: AppConfig,
    ) -> anyhow::Result<(RuntimeRegistry, Vec<fabric::RuntimeId>)> {
        let inference: Arc<dyn InferencePort> = Arc::new(NoopInference);
        let mut registry = RuntimeRegistry::new();
        let ids = super::register_goal_runtimes(
            &mut registry,
            &config,
            inference,
            &app.model_aliases,
            Vec::new(),
            Arc::new(NoopCapability),
            Arc::new(SystemClock::new()),
        )
        .await?;
        Ok((registry, ids))
    }

    #[tokio::test]
    async fn disabled_goal_runtime_registers_nothing() {
        let mut app = AppConfig::default();
        app.providers.push(provider("p"));
        let (registry, ids) = register(GoalRuntimeConfig::default(), app).await.unwrap();
        assert!(ids.is_empty());
        assert!(!registry.contains(&fabric::RuntimeId("deepseek-worker".into())));
    }

    #[tokio::test]
    async fn enabled_goal_runtime_rejects_missing_route_and_unknown_alias() {
        let mut app = AppConfig::default();
        app.providers.push(provider("p"));
        let missing = GoalRuntimeConfig {
            enabled: true,
            worker: Some(route("deepseek-worker", "p/model")),
            reviewer: None,
        };
        assert!(register(missing, app.clone())
            .await
            .unwrap_err()
            .to_string()
            .contains("reviewer routing is missing"));

        let unknown = GoalRuntimeConfig {
            enabled: true,
            worker: Some(route("deepseek-worker", "unknown-alias")),
            reviewer: Some(route("escalation-reviewer", "p/model")),
        };
        assert!(register(unknown, app)
            .await
            .unwrap_err()
            .to_string()
            .contains("model alias 'unknown-alias' not found"));
    }

    #[tokio::test]
    async fn same_provider_can_back_distinct_runtime_ids() {
        let mut app = AppConfig::default();
        app.providers.push(provider("shared"));
        let config = GoalRuntimeConfig {
            enabled: true,
            worker: Some(route("deepseek-worker", "shared/worker-model")),
            reviewer: Some(route("escalation-reviewer", "shared/reviewer-model")),
        };
        let (registry, ids) = register(config, app).await.unwrap();
        assert_eq!(ids.len(), 2);
        for id in ids {
            assert!(registry.contains(&id));
        }
    }

    #[tokio::test]
    async fn distinct_providers_register_worker_and_reviewer() {
        let mut app = AppConfig::default();
        app.providers.push(provider("worker-provider"));
        app.providers.push(provider("review-provider"));
        let config = GoalRuntimeConfig {
            enabled: true,
            worker: Some(route("deepseek-worker", "worker-provider/model")),
            reviewer: Some(route("escalation-reviewer", "review-provider/model")),
        };
        let (registry, ids) = register(config, app).await.unwrap();
        assert_eq!(
            ids,
            vec![
                fabric::RuntimeId("deepseek-worker".into()),
                fabric::RuntimeId("escalation-reviewer".into())
            ]
        );
        assert!(registry.contains(&ids[0]));
        assert!(registry.contains(&ids[1]));
    }

    #[tokio::test]
    async fn agent_profiles_resolve_model_tools_and_frontmatter_limits() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("reviewer.md"),
            "---\nname: reviewer\ndescription: review\ntools: [file_read, grep]\nmax_iterations: 3\n---\nReview evidence only.",
        )
        .unwrap();
        let mut app = AppConfig::default();
        app.providers.push(provider("shared"));
        app.agent.default_provider = Some("shared".into());
        app.agent.default_model = Some("model".into());
        let inference: Arc<dyn InferencePort> = Arc::new(NoopInference);
        let llm: Arc<dyn LlmProvider> = Arc::new(
            PortLlmProvider::resolve(inference.clone(), "shared/model")
                .await
                .unwrap(),
        );
        let definitions = ["file_read", "grep"]
            .into_iter()
            .map(|name| fabric::ToolDefinition {
                name: name.into(),
                description: name.into(),
                input_schema: serde_json::json!({"type":"object"}),
            })
            .collect::<Vec<_>>();
        let result = super::load_agent_profiles(
            directory.path(),
            inference,
            llm,
            &definitions,
            &definitions,
            &crate::composition::config::ExecutiveConfig::default(),
            &crate::composition::config::AgentProfilesConfig::default(),
        )
        .await
        .unwrap();
        let registry = result.registry;
        let profiles = result.profiles;
        let profile = profiles.get("reviewer").unwrap();
        assert_eq!(profile.allowed_tools, vec!["file_read", "grep"]);
        assert_eq!(profile.max_iterations, 3);
        assert_eq!(profile.max_tool_calls, 128);
        assert!(registry.resolve(&profile.id).is_ok());
    }

    #[tokio::test]
    async fn universal_tools_are_merged_into_every_profile() {
        let directory = tempfile::tempdir().unwrap();
        // A narrow profile that does NOT list git or task tools.
        std::fs::write(
            directory.path().join("narrow.md"),
            "---\nname: narrow\ndescription: narrow\ntools: [file_read]\n---\nNarrow.",
        )
        .unwrap();
        let inference: Arc<dyn InferencePort> = Arc::new(NoopInference);
        let llm: Arc<dyn LlmProvider> = Arc::new(
            PortLlmProvider::resolve(inference.clone(), "shared/model")
                .await
                .unwrap(),
        );
        // Catalog includes universal tools the profile did not declare.
        let definitions = ["file_read", "git_status", "git_reset", "task_create"]
            .into_iter()
            .map(|name| fabric::ToolDefinition {
                name: name.into(),
                description: name.into(),
                input_schema: serde_json::json!({"type":"object"}),
            })
            .collect::<Vec<_>>();
        let result = super::load_agent_profiles(
            directory.path(),
            inference,
            llm,
            &definitions,
            &definitions,
            &crate::composition::config::ExecutiveConfig::default(),
            &crate::composition::config::AgentProfilesConfig::default(),
        )
        .await
        .unwrap();
        let profile = result.profiles.get("narrow").unwrap();
        assert!(profile.allowed_tools.contains(&"file_read".to_string()));
        assert!(
            profile.allowed_tools.contains(&"git_status".to_string()),
            "git_status must be universally available regardless of profile"
        );
        assert!(
            !profile.allowed_tools.contains(&"git_reset".to_string()),
            "mutating git tools must not bypass profile and transaction governance"
        );
        assert!(
            profile.allowed_tools.contains(&"task_create".to_string()),
            "task_create must be universally available"
        );
    }

    #[tokio::test]
    async fn agent_profile_token_overrides_feed_adaptive_context_limits() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("compact.md"),
            "---\nname: compact\ndescription: compact\ntools: [file_read]\n---\nCompact safely.",
        )
        .unwrap();
        let inference: Arc<dyn InferencePort> = Arc::new(NoopInference);
        let llm: Arc<dyn LlmProvider> = Arc::new(
            PortLlmProvider::resolve(inference.clone(), "shared/model")
                .await
                .unwrap(),
        );
        let definitions = vec![fabric::ToolDefinition {
            name: "file_read".into(),
            description: "read".into(),
            input_schema: serde_json::json!({"type":"object"}),
        }];
        let mut profiles = crate::composition::config::AgentProfilesConfig::default();
        profiles.overrides.insert(
            "compact".into(),
            crate::composition::config::ProfileOverride {
                max_input_tokens: Some(8_000),
                max_output_tokens: Some(1_000),
                ..Default::default()
            },
        );

        let result = super::load_agent_profiles(
            directory.path(),
            inference,
            llm,
            &definitions,
            &definitions,
            &crate::composition::config::ExecutiveConfig::default(),
            &profiles,
        )
        .await
        .unwrap();

        let profile = result.profiles.get("compact").unwrap();
        assert_eq!(profile.max_input_tokens, 8_000);
        assert_eq!(profile.max_output_tokens, 1_000);
    }

    #[tokio::test]
    async fn agent_profile_config_rejects_unknown_default_and_override() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("reviewer.md"),
            "---\nname: reviewer\ndescription: review\ntools: [file_read]\n---\nReview.",
        )
        .unwrap();
        let inference: Arc<dyn InferencePort> = Arc::new(NoopInference);
        let llm: Arc<dyn LlmProvider> = Arc::new(
            PortLlmProvider::resolve(inference.clone(), "shared/model")
                .await
                .unwrap(),
        );
        let definitions = vec![fabric::ToolDefinition {
            name: "file_read".into(),
            description: "read".into(),
            input_schema: serde_json::json!({"type":"object"}),
        }];

        let mut unknown_default = crate::composition::config::AgentProfilesConfig {
            default: "missing".into(),
            ..Default::default()
        };
        assert!(super::load_agent_profiles(
            directory.path(),
            inference.clone(),
            llm.clone(),
            &definitions,
            &definitions,
            &crate::composition::config::ExecutiveConfig::default(),
            &unknown_default,
        )
        .await
        .is_err());

        unknown_default.default.clear();
        unknown_default.overrides.insert(
            "missing".into(),
            crate::composition::config::ProfileOverride::default(),
        );
        assert!(super::load_agent_profiles(
            directory.path(),
            inference,
            llm,
            &definitions,
            &definitions,
            &crate::composition::config::ExecutiveConfig::default(),
            &unknown_default,
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn agent_tool_registration_profile_can_reference_agent_spawn() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("orchestrator.md"),
            "---\nname: orchestrator\ndescription: orch\ntools: [file_read, agent_spawn]\n---\nI orchestrate.",
        )
        .unwrap();
        let inference: Arc<dyn InferencePort> = Arc::new(NoopInference);
        let llm: Arc<dyn LlmProvider> = Arc::new(
            PortLlmProvider::resolve(inference.clone(), "shared/model")
                .await
                .unwrap(),
        );

        let mut definitions: Vec<fabric::ToolDefinition> = vec![fabric::ToolDefinition {
            name: "file_read".into(),
            description: "read".into(),
            input_schema: serde_json::json!({"type": "object"}),
        }];
        definitions.extend(corpus::tools::tools::agent_control::AgentControlTools::definitions());

        let result = super::load_agent_profiles(
            directory.path(),
            inference,
            llm,
            &definitions,
            &definitions,
            &crate::composition::config::ExecutiveConfig::default(),
            &crate::composition::config::AgentProfilesConfig::default(),
        )
        .await
        .unwrap();
        let registry = result.registry;
        let profiles = result.profiles;

        let profile = profiles.get("orchestrator").unwrap();
        assert_eq!(profile.allowed_tools.len(), 2);
        assert!(profile.allowed_tools.contains(&"file_read".to_string()));
        assert!(profile.allowed_tools.contains(&"agent_spawn".to_string()));
        assert!(registry.resolve(&profile.id).is_ok());
    }

    #[tokio::test]
    async fn agent_tool_registration_unknown_tool_rejected() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("bad.md"),
            "---\nname: bad\ndescription: bad\ntools: [file_read, not_a_tool]\n---\nBad profile.",
        )
        .unwrap();
        let inference: Arc<dyn InferencePort> = Arc::new(NoopInference);
        let llm: Arc<dyn LlmProvider> = Arc::new(
            PortLlmProvider::resolve(inference.clone(), "shared/model")
                .await
                .unwrap(),
        );
        let definitions: Vec<fabric::ToolDefinition> = vec![fabric::ToolDefinition {
            name: "file_read".into(),
            description: "read".into(),
            input_schema: serde_json::json!({"type": "object"}),
        }];

        let result = super::load_agent_profiles(
            directory.path(),
            inference,
            llm,
            &definitions,
            &definitions,
            &crate::composition::config::ExecutiveConfig::default(),
            &crate::composition::config::AgentProfilesConfig::default(),
        )
        .await
        .unwrap();
        assert_eq!(result.quarantined.len(), 1);
        assert_eq!(result.quarantined[0].name, "bad");
        assert!(result.quarantined[0].reason.contains("not_a_tool"));
        assert!(
            result.profiles.is_empty(),
            "no valid profiles should remain when every profile is quarantined"
        );
    }

    #[tokio::test]
    async fn mixed_valid_and_invalid_profiles_degraded_not_fatal() {
        // Phase 3: invalid profiles are quarantined, valid ones remain active
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("reviewer.md"),
            "---\nname: reviewer\ndescription: review\ntools: [file_read]\n---\nReview.",
        )
        .unwrap();
        std::fs::write(
            directory.path().join("bad.md"),
            "---\nname: bad\ndescription: bad\ntools: [file_read, not_a_tool]\n---\nBad profile.",
        )
        .unwrap();
        let inference: Arc<dyn InferencePort> = Arc::new(NoopInference);
        let llm: Arc<dyn LlmProvider> = Arc::new(
            PortLlmProvider::resolve(inference.clone(), "shared/model")
                .await
                .unwrap(),
        );
        let definitions: Vec<fabric::ToolDefinition> = vec![fabric::ToolDefinition {
            name: "file_read".into(),
            description: "read".into(),
            input_schema: serde_json::json!({"type": "object"}),
        }];

        let result = super::load_agent_profiles(
            directory.path(),
            inference,
            llm,
            &definitions,
            &definitions,
            &crate::composition::config::ExecutiveConfig::default(),
            &crate::composition::config::AgentProfilesConfig::default(),
        )
        .await
        .unwrap();
        // Valid profile loaded, invalid profile quarantined
        assert_eq!(result.quarantined.len(), 1);
        assert_eq!(result.quarantined[0].name, "bad");
        assert!(
            result.profiles.contains_key("reviewer"),
            "valid profile must be loaded"
        );
    }

    #[tokio::test]
    async fn invalid_profile_error_messages_identify_the_failing_tool() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("bad.md"),
            "---\nname: bad-agent\ndescription: bad\ntools: [file_read, not_a_tool]\n---\nBad.",
        )
        .unwrap();
        let inference: Arc<dyn InferencePort> = Arc::new(NoopInference);
        let llm: Arc<dyn LlmProvider> = Arc::new(
            PortLlmProvider::resolve(inference.clone(), "shared/model")
                .await
                .unwrap(),
        );
        let definitions: Vec<fabric::ToolDefinition> = vec![fabric::ToolDefinition {
            name: "file_read".into(),
            description: "read".into(),
            input_schema: serde_json::json!({"type": "object"}),
        }];

        let result = super::load_agent_profiles(
            directory.path(),
            inference,
            llm,
            &definitions,
            &definitions,
            &crate::composition::config::ExecutiveConfig::default(),
            &crate::composition::config::AgentProfilesConfig::default(),
        )
        .await
        .unwrap();
        assert_eq!(
            result.quarantined.len(),
            1,
            "expected 1 quarantined profile"
        );
        let q = &result.quarantined[0];
        assert!(
            q.name.contains("bad-agent"),
            "quarantined name should contain 'bad-agent', got: {}",
            q.name
        );
        assert!(
            q.reason.contains("not_a_tool"),
            "reason should contain 'not_a_tool', got: {}",
            q.reason
        );
    }
}

#[cfg(test)]
mod combine_limits_tests {
    use super::combine_limits;

    #[test]
    fn zero_profile_zero_global_is_unlimited() {
        assert_eq!(combine_limits(0, 0), 0);
    }

    #[test]
    fn zero_profile_uses_global_cap() {
        assert_eq!(combine_limits(0, 50), 50);
    }

    #[test]
    fn zero_global_keeps_profile() {
        assert_eq!(combine_limits(20, 0), 20);
    }

    #[test]
    fn both_nonzero_takes_min() {
        assert_eq!(combine_limits(20, 50), 20);
        assert_eq!(combine_limits(80, 50), 50);
    }
}
