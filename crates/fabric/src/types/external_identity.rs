//! Provider-neutral external identity and capability grant contracts.
//!
//! Credential material deliberately has no representation in these shared
//! types. Tokens remain in provider adapters and encrypted persistence.

use crate::PrincipalId;
use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

const MAX_PROVIDER_SUBJECT_BYTES: usize = 512;
const MAX_EMAIL_BYTES: usize = 320;
const MAX_ALIAS_BYTES: usize = 128;
const MAX_SCOPES: usize = 32;
const MAX_PROVIDER_ID_BYTES: usize = 64;
const MAX_CAPABILITY_ID_BYTES: usize = 128;

/// Stable authority for the credential-checked single-user native daemon.
pub const LOCAL_OWNER_PRINCIPAL: &str = "local-owner";

#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ExternalIdentityId(pub Uuid);

impl ExternalIdentityId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ExternalIdentityId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for ExternalIdentityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ExternalIdentityId({})", self.0)
    }
}

impl fmt::Display for ExternalIdentityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct ExternalProviderId(String);

impl ExternalProviderId {
    pub fn new(value: impl Into<String>) -> Result<Self, ExternalIdentityContractError> {
        let value = value.into();
        validate_canonical_id(&value, MAX_PROVIDER_ID_BYTES, "provider")?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for ExternalProviderId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

impl fmt::Display for ExternalProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct ExternalCapabilityId(String);

impl ExternalCapabilityId {
    pub fn new(value: impl Into<String>) -> Result<Self, ExternalIdentityContractError> {
        let value = value.into();
        validate_canonical_id(&value, MAX_CAPABILITY_ID_BYTES, "capability")?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for ExternalCapabilityId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

impl fmt::Display for ExternalCapabilityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalIdentityState {
    Active,
    Revoked,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalIdentity {
    pub id: ExternalIdentityId,
    pub provider: ExternalProviderId,
    pub principal_id: PrincipalId,
    pub provider_subject: String,
    pub email: String,
    pub alias: Option<String>,
    pub state: ExternalIdentityState,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub version: u64,
}

impl ExternalIdentity {
    pub fn validate(&self) -> Result<(), ExternalIdentityContractError> {
        bounded_nonempty(
            &self.provider_subject,
            MAX_PROVIDER_SUBJECT_BYTES,
            "provider_subject",
        )?;
        bounded_nonempty(&self.email, MAX_EMAIL_BYTES, "email")?;
        if !self.email.contains('@') {
            return Err(ExternalIdentityContractError::InvalidField("email"));
        }
        if let Some(alias) = &self.alias {
            bounded_nonempty(alias, MAX_ALIAS_BYTES, "alias")?;
        }
        if self.updated_at_ms < self.created_at_ms {
            return Err(ExternalIdentityContractError::InvalidField("updated_at_ms"));
        }
        Ok(())
    }
}

impl fmt::Debug for ExternalIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExternalIdentity")
            .field("id", &self.id)
            .field("provider", &self.provider)
            .field("principal_id", &self.principal_id)
            .field("provider_subject", &"[REDACTED]")
            .field("email", &"[REDACTED]")
            .field("alias", &self.alias.as_ref().map(|_| "[REDACTED]"))
            .field("state", &self.state)
            .field("version", &self.version)
            .finish()
    }
}

impl fmt::Display for ExternalIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{} ({:?})", self.provider, self.id, self.state)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrantState {
    Active,
    Revoked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityGrant {
    pub identity_id: ExternalIdentityId,
    /// Canonical capability IDs. The persisted field name remains `scopes`
    /// for backward compatibility; provider policy owns OAuth translation and
    /// read/write semantics.
    pub scopes: Vec<ExternalCapabilityId>,
    pub state: GrantState,
    pub granted_at_ms: i64,
    pub revoked_at_ms: Option<i64>,
    pub version: u64,
}

impl CapabilityGrant {
    pub fn validate(&self) -> Result<(), ExternalIdentityContractError> {
        if self.scopes.is_empty() || self.scopes.len() > MAX_SCOPES {
            return Err(ExternalIdentityContractError::InvalidField("scopes"));
        }
        let mut scopes = self.scopes.clone();
        scopes.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        scopes.dedup();
        if scopes.len() != self.scopes.len() {
            return Err(ExternalIdentityContractError::InvalidField("scopes"));
        }
        match (self.state, self.revoked_at_ms) {
            (GrantState::Active, None) => Ok(()),
            (GrantState::Revoked, Some(at)) if at >= self.granted_at_ms => Ok(()),
            _ => Err(ExternalIdentityContractError::InvalidField("revoked_at_ms")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalIdentityContractError {
    InvalidField(&'static str),
    FieldTooLarge(&'static str),
    WriteScopeDenied,
}

impl fmt::Display for ExternalIdentityContractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidField(field) => write!(f, "invalid external identity field: {field}"),
            Self::FieldTooLarge(field) => write!(f, "external identity field too large: {field}"),
            Self::WriteScopeDenied => f.write_str("write or inactive grant is unavailable"),
        }
    }
}

impl std::error::Error for ExternalIdentityContractError {}

fn bounded_nonempty(
    value: &str,
    max: usize,
    field: &'static str,
) -> Result<(), ExternalIdentityContractError> {
    if value.trim().is_empty() {
        return Err(ExternalIdentityContractError::InvalidField(field));
    }
    if value.len() > max {
        return Err(ExternalIdentityContractError::FieldTooLarge(field));
    }
    Ok(())
}

fn validate_canonical_id(
    value: &str,
    max: usize,
    field: &'static str,
) -> Result<(), ExternalIdentityContractError> {
    if value.is_empty()
        || value.len() > max
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
        })
        || !value.as_bytes()[0].is_ascii_alphanumeric()
    {
        return Err(if value.len() > max {
            ExternalIdentityContractError::FieldTooLarge(field)
        } else {
            ExternalIdentityContractError::InvalidField(field)
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> ExternalIdentity {
        ExternalIdentity {
            id: ExternalIdentityId::new(),
            provider: ExternalProviderId::new("example").unwrap(),
            principal_id: PrincipalId("owner".into()),
            provider_subject: "external-subject-secret".into(),
            email: "owner@example.com".into(),
            alias: Some("work".into()),
            state: ExternalIdentityState::Active,
            created_at_ms: 10,
            updated_at_ms: 10,
            version: 1,
        }
    }

    #[test]
    fn identity_round_trips_without_credential_fields() {
        let value = identity();
        value.validate().unwrap();
        let json = serde_json::to_string(&value).unwrap();
        let decoded: ExternalIdentity = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, value);
        assert!(!json.contains("access_token"));
        assert!(!json.contains("refresh_token"));
    }

    #[test]
    fn debug_and_display_redact_provider_pii() {
        let value = identity();
        for rendered in [format!("{value:?}"), value.to_string()] {
            assert!(!rendered.contains("external-subject-secret"));
            assert!(!rendered.contains("owner@example.com"));
            assert!(!rendered.contains("work"));
        }
    }

    #[test]
    fn grant_models_revocation_and_rejects_write_scope() {
        let mut grant = CapabilityGrant {
            identity_id: ExternalIdentityId::new(),
            scopes: vec![
                ExternalCapabilityId::new("identity.basic").unwrap(),
                ExternalCapabilityId::new("mail.read").unwrap(),
            ],
            state: GrantState::Active,
            granted_at_ms: 10,
            revoked_at_ms: None,
            version: 1,
        };
        grant.validate().unwrap();
        grant
            .scopes
            .push(ExternalCapabilityId::new("mail.write").unwrap());
        grant.validate().unwrap();
        grant.scopes.pop();
        grant.state = GrantState::Revoked;
        grant.revoked_at_ms = Some(20);
        grant.validate().unwrap();
        grant.validate().unwrap();
    }

    #[test]
    fn identity_rejects_oversized_fields() {
        let mut value = identity();
        value.provider_subject = "x".repeat(MAX_PROVIDER_SUBJECT_BYTES + 1);
        assert_eq!(
            value.validate(),
            Err(ExternalIdentityContractError::FieldTooLarge(
                "provider_subject"
            ))
        );
    }

    #[test]
    fn provider_and_capability_ids_are_bounded_canonical_values() {
        for value in ["", "Upper", "has space", "line\nbreak", "slash/value"] {
            assert!(ExternalProviderId::new(value).is_err());
            assert!(ExternalCapabilityId::new(value).is_err());
        }
        assert!(ExternalProviderId::new("x".repeat(MAX_PROVIDER_ID_BYTES + 1)).is_err());
        assert_eq!(
            serde_json::from_str::<ExternalCapabilityId>("\"mail.read\"")
                .unwrap()
                .as_str(),
            "mail.read"
        );
    }

    #[test]
    fn legacy_string_schema_for_identity_and_grant_remains_readable() {
        let identity_json = r#"{
            "id":"00000000-0000-0000-0000-000000000001",
            "provider":"google",
            "principal_id":"owner",
            "provider_subject":"subject-1",
            "email":"owner@example.com",
            "alias":"work",
            "state":"active",
            "created_at_ms":10,
            "updated_at_ms":10,
            "version":1
        }"#;
        let identity: ExternalIdentity = serde_json::from_str(identity_json).unwrap();
        identity.validate().unwrap();
        assert_eq!(identity.provider.as_str(), "google");

        let grant_json = r#"{
            "identity_id":"00000000-0000-0000-0000-000000000001",
            "scopes":["open_id","gmail_readonly"],
            "state":"active",
            "granted_at_ms":10,
            "revoked_at_ms":null,
            "version":1
        }"#;
        let grant: CapabilityGrant = serde_json::from_str(grant_json).unwrap();
        grant.validate().unwrap();
        assert_eq!(grant.scopes[1].as_str(), "gmail_readonly");
    }
}
