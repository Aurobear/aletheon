//! Typed bootstrap configuration and secret references for optional integrations.

use std::fmt;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SecretRef {
    /// Environment variable containing the secret value. The variable name is
    /// configuration; its value never becomes part of AppConfig diagnostics.
    pub env: String,
}

impl fmt::Debug for SecretRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecretRef")
            .field("env", &self.env)
            .finish()
    }
}

#[derive(Clone)]
pub struct SecretValue(String);

impl SecretValue {
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretValue([REDACTED])")
    }
}

pub trait CredentialResolver {
    fn resolve(&self, reference: &SecretRef) -> Result<SecretValue>;
}

#[derive(Debug, Default)]
pub struct EnvironmentCredentialResolver;

impl CredentialResolver for EnvironmentCredentialResolver {
    fn resolve(&self, reference: &SecretRef) -> Result<SecretValue> {
        if reference.env.trim().is_empty() {
            bail!("secret reference contains an empty environment variable name");
        }
        let value = std::env::var(&reference.env).with_context(|| {
            format!("credential reference env:{} is unavailable", reference.env)
        })?;
        if value.is_empty() {
            bail!(
                "credential reference env:{} resolved to an empty value",
                reference.env
            );
        }
        Ok(SecretValue(value))
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct RuntimeBootstrapConfig {
    pub working_dir: Option<PathBuf>,
    pub data_dir: Option<PathBuf>,
    pub sandbox_preference: Option<String>,
    pub conscious_arbitration_mode: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OAuthClientType {
    #[default]
    Public,
    Confidential,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct GoogleIntegrationConfig {
    pub client_type: OAuthClientType,
    pub client_id: Option<String>,
    pub client_secret: Option<SecretRef>,
    pub redirect_uri: Option<String>,
    pub drive_sync_enabled: bool,
    pub drive_file_ids: Vec<String>,
    pub gmail_ingress_policy_file: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct SearchIntegrationConfig {
    pub enabled: bool,
    pub api_url: Option<String>,
    pub api_key: Option<SecretRef>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct IntegrationsConfig {
    pub google: GoogleIntegrationConfig,
    pub search: SearchIntegrationConfig,
    /// Optional embodiment provider config. Defaults to simulator when absent.
    pub embodiment: Option<EmbodimentProviderConfig>,
}

/// Tagged configuration for the embodied device provider.
///
/// Default when absent: `Simulator { device_id: "bot" }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EmbodimentProviderConfig {
    /// Deterministic simulator (default).
    Simulator {
        /// Device ID exposed to the model.
        #[serde(default = "default_sim_device_id")]
        device_id: String,
    },
    /// gRPC embodiment gateway (e.g. Kuavo MuJoCo bridge).
    Grpc {
        /// Device ID exposed to the model.
        device_id: String,
        /// gRPC endpoint URL (e.g. "http://127.0.0.1:50051").
        endpoint: String,
        /// Connection timeout in milliseconds.
        #[serde(default = "default_connect_timeout_ms")]
        connect_timeout_ms: u64,
        /// Per-RPC request timeout in milliseconds.
        #[serde(default = "default_request_timeout_ms")]
        request_timeout_ms: u64,
    },
}

fn default_sim_device_id() -> String {
    "bot".into()
}

fn default_connect_timeout_ms() -> u64 {
    5000
}

fn default_request_timeout_ms() -> u64 {
    30000
}

impl Default for EmbodimentProviderConfig {
    fn default() -> Self {
        Self::Simulator {
            device_id: "bot".into(),
        }
    }
}

#[derive(Clone)]
pub struct ResolvedGoogleIntegration {
    pub client_id: String,
    pub client_secret: Option<SecretValue>,
    pub redirect_uri: String,
    pub drive_sync_enabled: bool,
    pub drive_file_ids: Vec<String>,
    pub gmail_ingress_policy_file: Option<PathBuf>,
}

impl fmt::Debug for ResolvedGoogleIntegration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResolvedGoogleIntegration")
            .field("client_id", &self.client_id)
            .field("client_secret", &self.client_secret)
            .field("redirect_uri", &self.redirect_uri)
            .field("drive_sync_enabled", &self.drive_sync_enabled)
            .field("drive_file_ids", &self.drive_file_ids)
            .field("gmail_ingress_policy_file", &self.gmail_ingress_policy_file)
            .finish()
    }
}

#[derive(Clone)]
pub struct ResolvedSearchIntegration {
    pub api_url: String,
    pub api_key: SecretValue,
}

impl fmt::Debug for ResolvedSearchIntegration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResolvedSearchIntegration")
            .field("api_url", &self.api_url)
            .field("api_key", &self.api_key)
            .finish()
    }
}

#[derive(Debug, Clone, Default)]
pub struct ResolvedIntegrations {
    pub google: Option<ResolvedGoogleIntegration>,
    pub search: Option<ResolvedSearchIntegration>,
}

impl IntegrationsConfig {
    pub fn preflight(
        &self,
        google_enabled: bool,
        resolver: &dyn CredentialResolver,
    ) -> Result<ResolvedIntegrations> {
        let google = if google_enabled {
            let client_id = required(&self.google.client_id, "integrations.google.client_id")?;
            let redirect_uri = required(
                &self.google.redirect_uri,
                "integrations.google.redirect_uri",
            )?;
            let client_secret = match (&self.google.client_type, &self.google.client_secret) {
                (OAuthClientType::Confidential, None) => {
                    bail!("integrations.google.client_secret is required for confidential clients")
                }
                (_, Some(reference)) => Some(resolver.resolve(reference).with_context(|| {
                    "resolve integrations.google.client_secret credential reference"
                })?),
                (OAuthClientType::Public, None) => None,
            };
            Some(ResolvedGoogleIntegration {
                client_id,
                client_secret,
                redirect_uri,
                drive_sync_enabled: self.google.drive_sync_enabled,
                drive_file_ids: self.google.drive_file_ids.clone(),
                gmail_ingress_policy_file: self.google.gmail_ingress_policy_file.clone(),
            })
        } else {
            None
        };

        let search = if self.search.enabled {
            let api_url = required(&self.search.api_url, "integrations.search.api_url")?;
            let api_key = self
                .search
                .api_key
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("integrations.search.api_key is required"))?;
            Some(ResolvedSearchIntegration {
                api_url,
                api_key: resolver
                    .resolve(api_key)
                    .context("resolve integrations.search.api_key credential reference")?,
            })
        } else {
            None
        };
        Ok(ResolvedIntegrations { google, search })
    }
}

fn required(value: &Option<String>, path: &str) -> Result<String> {
    value
        .as_ref()
        .filter(|value| !value.trim().is_empty())
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("{path} is required when the integration is enabled"))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FixedResolver;

    impl CredentialResolver for FixedResolver {
        fn resolve(&self, reference: &SecretRef) -> Result<SecretValue> {
            Ok(SecretValue(format!("value-for-{}", reference.env)))
        }
    }

    #[test]
    fn disabled_integrations_require_nothing() {
        assert!(IntegrationsConfig::default()
            .preflight(false, &FixedResolver)
            .is_ok());
    }

    #[test]
    fn public_google_client_does_not_require_secret() {
        let config = IntegrationsConfig {
            google: GoogleIntegrationConfig {
                client_id: Some("id".into()),
                redirect_uri: Some("http://localhost/callback".into()),
                ..GoogleIntegrationConfig::default()
            },
            ..IntegrationsConfig::default()
        };
        assert!(config
            .preflight(true, &FixedResolver)
            .unwrap()
            .google
            .is_some());
    }

    #[test]
    fn confidential_google_client_requires_secret_reference() {
        let config = IntegrationsConfig {
            google: GoogleIntegrationConfig {
                client_type: OAuthClientType::Confidential,
                client_id: Some("id".into()),
                redirect_uri: Some("http://localhost/callback".into()),
                ..GoogleIntegrationConfig::default()
            },
            ..IntegrationsConfig::default()
        };
        assert!(config
            .preflight(true, &FixedResolver)
            .unwrap_err()
            .to_string()
            .contains("client_secret"));
    }

    #[test]
    fn enabled_search_reports_missing_typed_field() {
        let config = IntegrationsConfig {
            search: SearchIntegrationConfig {
                enabled: true,
                ..SearchIntegrationConfig::default()
            },
            ..IntegrationsConfig::default()
        };
        assert_eq!(
            config
                .preflight(false, &FixedResolver)
                .unwrap_err()
                .to_string(),
            "integrations.search.api_url is required when the integration is enabled"
        );
    }

    #[test]
    fn secret_debug_never_exposes_value() {
        assert_eq!(
            format!("{:?}", SecretValue("sensitive".into())),
            "SecretValue([REDACTED])"
        );
    }
}
