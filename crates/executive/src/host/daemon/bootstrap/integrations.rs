//! Typed construction units and degradation policy for optional integrations.

use std::path::Path;
use std::sync::Arc;

use corpus::tools::tools::ToolRegistry;
use fabric::Clock;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use super::google::ConfiguredGoogleReadTools;

pub(super) struct GoogleCompositionInput<'a> {
    pub(super) tools: &'a mut ToolRegistry,
    pub(super) objective_db_path: &'a Path,
    pub(super) clock: Arc<dyn Clock>,
    pub(super) cancel: &'a CancellationToken,
    pub(super) artifact_root: &'a Path,
    pub(super) storage_quota: Option<crate::application::storage_quota::StorageQuota>,
    pub(super) config: Option<&'a crate::composition::config::ResolvedGoogleIntegration>,
}

#[derive(Default)]
pub(super) struct GoogleComposition {
    pub(super) integration: Option<Arc<crate::adapters::external::GoogleIntegration>>,
    pub(super) sync: Option<crate::adapters::google::GoogleSyncHandle>,
    pub(super) sync_store: Option<Arc<std::sync::Mutex<crate::adapters::google::GoogleSyncStore>>>,
    pub(super) gmail_ingress: Option<Arc<crate::adapters::channel::gmail::GmailGoalEventIngress>>,
    pub(super) degraded_reason: Option<String>,
}

fn optional_result(result: anyhow::Result<Option<ConfiguredGoogleReadTools>>) -> GoogleComposition {
    match result {
        Ok(Some((integration, sync, sync_store, gmail_ingress))) => GoogleComposition {
            integration: Some(integration),
            sync: Some(sync),
            sync_store: Some(sync_store),
            gmail_ingress,
            degraded_reason: None,
        },
        Ok(None) => GoogleComposition::default(),
        Err(error) => GoogleComposition {
            degraded_reason: Some(error.to_string()),
            ..GoogleComposition::default()
        },
    }
}

pub(super) fn compose_google(input: GoogleCompositionInput<'_>) -> GoogleComposition {
    let composition = optional_result(super::google::register_configured_external_read_tools(
        input.tools,
        input.objective_db_path,
        input.clock,
        input.cancel,
        input.artifact_root,
        input.storage_quota,
        input.config,
    ));
    if let Some(reason) = composition.degraded_reason.as_deref() {
        warn!(error = reason, "Google read integration disabled");
    }
    composition
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_optional_integration_is_not_degraded() {
        let result = optional_result(Ok(None));
        assert!(result.integration.is_none());
        assert!(result.degraded_reason.is_none());
    }

    #[test]
    fn optional_integration_failure_degrades_without_failing_bootstrap() {
        let result = optional_result(Err(anyhow::anyhow!("credential vault unavailable")));
        assert!(result.integration.is_none());
        assert_eq!(
            result.degraded_reason.as_deref(),
            Some("credential vault unavailable")
        );
    }
}
