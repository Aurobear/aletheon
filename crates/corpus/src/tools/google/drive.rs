//! Minimal read-only Google Drive API adapter used by delta synchronization.

use super::{GoogleApiClient, GoogleApiError};
use fabric::{ExternalCapabilityId, ExternalIdentityId, PrincipalId};
use serde::de::DeserializeOwned;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone)]
pub struct GoogleDriveAdapter {
    pub(crate) client: GoogleApiClient,
}

impl GoogleDriveAdapter {
    pub fn new(client: GoogleApiClient) -> Self {
        Self { client }
    }

    pub(crate) async fn get_json<T: DeserializeOwned>(
        &self,
        principal: &PrincipalId,
        account: ExternalIdentityId,
        path: &str,
        query: &[(&str, &str)],
        cancel: &CancellationToken,
    ) -> Result<T, GoogleApiError> {
        let mut url = reqwest::Url::parse(&format!(
            "{}/{}",
            self.client.endpoints().drive_base.trim_end_matches('/'),
            path.trim_start_matches('/')
        ))
        .map_err(|_| GoogleApiError::InvalidRequest)?;
        url.query_pairs_mut().extend_pairs(query.iter().copied());
        self.client
            .get_json(
                principal,
                account,
                ExternalCapabilityId::new("file.read").unwrap(),
                url,
                cancel,
            )
            .await
    }

    pub(crate) async fn download(
        &self,
        principal: &PrincipalId,
        account: ExternalIdentityId,
        file_id: &str,
        max_bytes: usize,
        cancel: &CancellationToken,
    ) -> Result<Vec<u8>, GoogleApiError> {
        let mut url = reqwest::Url::parse(&format!(
            "{}/files/{}",
            self.client.endpoints().drive_base.trim_end_matches('/'),
            file_id
        ))
        .map_err(|_| GoogleApiError::InvalidRequest)?;
        url.query_pairs_mut()
            .append_pair("alt", "media")
            .append_pair("supportsAllDrives", "true");
        self.client
            .get_bounded_bytes(
                principal,
                account,
                ExternalCapabilityId::new("file.read").unwrap(),
                url,
                max_bytes,
                cancel,
            )
            .await
    }
}
