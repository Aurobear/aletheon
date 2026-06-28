use std::sync::Arc;

use anyhow::Result;

use crate::config::{ProviderConfig, Transport};
use super::anthropic::AnthropicProvider;
use super::ollama::OllamaProvider;
use super::openai_provider::OpenAiProvider;
use super::provider::LlmProvider;

/// Auto-detect provider kind from base_url when transport is `Auto`.
///
/// Heuristics:
/// - URL containing `anthropic.com` or ending with `/anthropic` -> "anthropic"
/// - URL containing `localhost:11434` or `127.0.0.1:11434` -> "ollama"
/// - Everything else -> "openai"
fn detect_provider_kind(base_url: &str) -> &str {
    let normalized = base_url.trim().to_lowercase();
    if normalized.contains("anthropic.com") || normalized.ends_with("/anthropic") {
        "anthropic"
    } else if normalized.contains("localhost:11434") || normalized.contains("127.0.0.1:11434") {
        "ollama"
    } else {
        "openai"
    }
}

/// Create an `LlmProvider` from a `ProviderConfig` and model name.
///
/// Provider selection logic:
/// - `Transport::Anthropic` -> `AnthropicProvider`
/// - `Transport::Openai` -> `OpenAiProvider` (works with any OpenAI-compatible API)
/// - `Transport::Auto` -> auto-detect from `base_url`:
///   - `/anthropic` suffix -> `AnthropicProvider`
///   - `localhost:11434` -> `OllamaProvider` (native Ollama `/api/chat` endpoint)
///   - Everything else -> `OpenAiProvider`
pub fn create_provider(config: &ProviderConfig, model: &str) -> Result<Arc<dyn LlmProvider>> {
    let api_key = resolve_api_key(config)?;

    match &config.transport {
        Transport::Anthropic => {
            let provider = AnthropicProvider::new(&api_key, model)
                .with_base_url(&config.base_url);
            Ok(Arc::new(provider))
        }
        Transport::Openai => {
            let provider = OpenAiProvider::new(&api_key, model, &config.base_url);
            Ok(Arc::new(provider))
        }
        Transport::Auto => {
            let kind = detect_provider_kind(&config.base_url);
            match kind {
                "anthropic" => {
                    let provider = AnthropicProvider::new(&api_key, model)
                        .with_base_url(&config.base_url);
                    Ok(Arc::new(provider))
                }
                "ollama" => {
                    let provider = OllamaProvider::new(model)
                        .with_base_url(&config.base_url);
                    Ok(Arc::new(provider))
                }
                _ => {
                    let provider = OpenAiProvider::new(&api_key, model, &config.base_url);
                    Ok(Arc::new(provider))
                }
            }
        }
    }
}

/// Create a provider by kind string (for explicit configuration).
///
/// Supported kinds: "anthropic", "openai", "ollama".
pub fn create_provider_by_kind(
    kind: &str,
    config: &ProviderConfig,
    model: &str,
) -> Result<Arc<dyn LlmProvider>> {
    let api_key = resolve_api_key(config)?;

    match kind {
        "anthropic" => {
            let provider = AnthropicProvider::new(&api_key, model)
                .with_base_url(&config.base_url);
            Ok(Arc::new(provider))
        }
        "openai" => {
            let provider = OpenAiProvider::new(&api_key, model, &config.base_url);
            Ok(Arc::new(provider))
        }
        "ollama" => {
            let provider = OllamaProvider::new(model)
                .with_base_url(&config.base_url);
            Ok(Arc::new(provider))
        }
        _ => anyhow::bail!(
            "Unknown provider kind: '{}'. Supported: anthropic, openai, ollama",
            kind
        ),
    }
}

/// Resolve API key: config value first, then env var `<NAME>_API_KEY`.
///
/// Returns an error if no key is found and the provider requires one.
/// Ollama (local) is exempt — it doesn't need an API key.
fn resolve_api_key(config: &ProviderConfig) -> Result<String> {
    if !config.api_key.is_empty() {
        return Ok(config.api_key.clone());
    }
    let env_name = format!(
        "{}_API_KEY",
        config.name.to_uppercase().replace('-', "_")
    );
    match std::env::var(&env_name) {
        Ok(key) if !key.is_empty() => Ok(key),
        _ => {
            // Ollama doesn't need an API key
            let base_lower = config.base_url.to_lowercase();
            if base_lower.contains("localhost:11434") || base_lower.contains("127.0.0.1:11434") {
                Ok(String::new())
            } else {
                anyhow::bail!(
                    "API key not found for provider '{}'. \
                     Set {} in your environment or add api_key to config.",
                    config.name,
                    env_name
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_provider_kind_anthropic() {
        assert_eq!(
            detect_provider_kind("https://api.example.com/anthropic"),
            "anthropic"
        );
    }

    #[test]
    fn test_detect_provider_kind_anthropic_official_url() {
        assert_eq!(
            detect_provider_kind("https://api.anthropic.com"),
            "anthropic"
        );
    }

    #[test]
    fn test_detect_provider_kind_anthropic_with_path() {
        assert_eq!(
            detect_provider_kind("https://api.example.com/anthropic"),
            "anthropic"
        );
    }

    #[test]
    fn test_detect_provider_kind_ollama() {
        assert_eq!(
            detect_provider_kind("http://localhost:11434"),
            "ollama"
        );
        assert_eq!(
            detect_provider_kind("http://127.0.0.1:11434"),
            "ollama"
        );
    }

    #[test]
    fn test_detect_provider_kind_openai_default() {
        assert_eq!(
            detect_provider_kind("https://api.openai.com"),
            "openai"
        );
        assert_eq!(
            detect_provider_kind("https://api.deepseek.com"),
            "openai"
        );
    }

    #[test]
    fn test_create_provider_anthropic_transport() {
        let config = ProviderConfig {
            name: "anthropic".to_string(),
            base_url: "https://api.anthropic.com".to_string(),
            api_key: "sk-test".to_string(),
            transport: Transport::Anthropic,
            models: vec!["claude-sonnet-4-20250514".to_string()],
        };
        let provider = create_provider(&config, "claude-sonnet-4-20250514");
        assert!(provider.is_ok());
        assert_eq!(provider.unwrap().name(), "claude-sonnet-4-20250514");
    }

    #[test]
    fn test_create_provider_openai_transport() {
        let config = ProviderConfig {
            name: "openai".to_string(),
            base_url: "https://api.openai.com".to_string(),
            api_key: "sk-test".to_string(),
            transport: Transport::Openai,
            models: vec!["gpt-4o".to_string()],
        };
        let provider = create_provider(&config, "gpt-4o");
        assert!(provider.is_ok());
        assert_eq!(provider.unwrap().name(), "gpt-4o");
    }

    #[test]
    fn test_create_provider_auto_ollama() {
        let config = ProviderConfig {
            name: "ollama".to_string(),
            base_url: "http://localhost:11434".to_string(),
            api_key: String::new(),
            transport: Transport::Auto,
            models: vec!["qwen3:8b".to_string()],
        };
        let provider = create_provider(&config, "qwen3:8b");
        assert!(provider.is_ok());
        assert_eq!(provider.unwrap().name(), "qwen3:8b");
    }

    #[test]
    fn test_create_provider_by_kind_unknown() {
        let config = ProviderConfig {
            name: "test".to_string(),
            base_url: "http://localhost".to_string(),
            api_key: String::new(),
            transport: Transport::Auto,
            models: vec![],
        };
        let result = create_provider_by_kind("unknown", &config, "model");
        assert!(result.is_err());
    }

    #[test]
    fn test_create_provider_by_kind_ollama() {
        let config = ProviderConfig {
            name: "ollama".to_string(),
            base_url: "http://localhost:11434".to_string(),
            api_key: String::new(),
            transport: Transport::Auto,
            models: vec!["llama3".to_string()],
        };
        let provider = create_provider_by_kind("ollama", &config, "llama3");
        assert!(provider.is_ok());
    }

    #[test]
    fn test_resolve_api_key_from_config() {
        let config = ProviderConfig {
            name: "test".to_string(),
            base_url: String::new(),
            api_key: "sk-secret".to_string(),
            transport: Transport::Auto,
            models: vec![],
        };
        assert_eq!(resolve_api_key(&config).unwrap(), "sk-secret");
    }

    #[test]
    fn test_resolve_api_key_missing_returns_error() {
        let config = ProviderConfig {
            name: "anthropic".to_string(),
            base_url: "https://api.anthropic.com".to_string(),
            api_key: String::new(),
            transport: Transport::Auto,
            models: vec![],
        };
        // Remove env var if set
        std::env::remove_var("ANTHROPIC_API_KEY");
        let result = resolve_api_key(&config);
        assert!(result.is_err(), "should fail when API key is missing");
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("ANTHROPIC_API_KEY"), "error should mention the env var name: {}", err_msg);
    }

    #[test]
    fn test_resolve_api_key_ollama_no_key_ok() {
        let config = ProviderConfig {
            name: "ollama".to_string(),
            base_url: "http://localhost:11434".to_string(),
            api_key: String::new(),
            transport: Transport::Auto,
            models: vec![],
        };
        let result = resolve_api_key(&config);
        assert!(result.is_ok(), "ollama should not require API key");
        assert_eq!(result.unwrap(), "");
    }
}
