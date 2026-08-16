//! Governed-review service composition.

use std::path::Path;
use std::sync::Arc;

use anyhow::Context;

use crate::config::GovernedReviewSettings;
use crate::wiring::governed_review::{GovernedReviewService, GovernedReviewStore};
use cognit::ports::inference::InferencePort;

pub(super) async fn compose_governed_review(
    settings: &GovernedReviewSettings,
    data_dir: &Path,
    inference: Arc<dyn InferencePort>,
) -> anyhow::Result<Option<Arc<GovernedReviewService>>> {
    settings.validate()?;
    if !settings.enabled {
        return Ok(None);
    }

    let store =
        Arc::new(GovernedReviewStore::open(data_dir).context("opening governed review store")?);
    let requested_model = if settings.model == "default" {
        // The machine inference registry owns the effective default. A
        // daemon-facing model id may itself contain '/', so it must not be
        // reinterpreted here as a provider-qualified spec.
        ""
    } else {
        settings.model.as_str()
    };
    let model = inference
        .capabilities(requested_model)
        .await
        .context("resolving governed review model through machine inference")?
        .model_spec;
    let service = GovernedReviewService::new(
        store,
        inference,
        model,
        settings.limits()?,
        uuid::Uuid::new_v4().to_string(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX),
    )
    .context("constructing governed review service")?;
    service.recover().await;
    Ok(Some(service))
}
