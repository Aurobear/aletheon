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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentityProvider {
    Google,
}

impl fmt::Display for IdentityProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Google => "google",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalScope {
    OpenId,
    UserInfoEmail,
    GmailReadonly,
    CalendarReadonly,
    GmailModify,
    GmailSend,
    CalendarEventsWrite,
}

impl ExternalScope {
    pub const fn oauth_name(self) -> &'static str {
        match self {
            Self::OpenId => "openid",
            Self::UserInfoEmail => "https://www.googleapis.com/auth/userinfo.email",
            Self::GmailReadonly => "https://www.googleapis.com/auth/gmail.readonly",
            Self::CalendarReadonly => "https://www.googleapis.com/auth/calendar.readonly",
            Self::GmailModify => "https://www.googleapis.com/auth/gmail.modify",
            Self::GmailSend => "https://www.googleapis.com/auth/gmail.send",
            Self::CalendarEventsWrite => "https://www.googleapis.com/auth/calendar.events",
        }
    }

    pub const fn is_write(self) -> bool {
        matches!(
            self,
            Self::GmailModify | Self::GmailSend | Self::CalendarEventsWrite
        )
    }

    pub const fn is_m6_allowed(self) -> bool {
        matches!(
            self,
            Self::OpenId | Self::UserInfoEmail | Self::GmailReadonly | Self::CalendarReadonly
        )
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
    pub provider: IdentityProvider,
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
    pub scopes: Vec<ExternalScope>,
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
        scopes.sort_by_key(|scope| scope.oauth_name());
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

    pub fn validate_m6_read_only(&self) -> Result<(), ExternalIdentityContractError> {
        self.validate()?;
        if self.state != GrantState::Active
            || self.scopes.iter().any(|scope| !scope.is_m6_allowed())
        {
            return Err(ExternalIdentityContractError::WriteScopeDenied);
        }
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> ExternalIdentity {
        ExternalIdentity {
            id: ExternalIdentityId::new(),
            provider: IdentityProvider::Google,
            principal_id: PrincipalId("owner".into()),
            provider_subject: "google-subject-secret".into(),
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
            assert!(!rendered.contains("google-subject-secret"));
            assert!(!rendered.contains("owner@example.com"));
            assert!(!rendered.contains("work"));
        }
    }

    #[test]
    fn grant_models_revocation_and_rejects_write_scope() {
        let mut grant = CapabilityGrant {
            identity_id: ExternalIdentityId::new(),
            scopes: vec![ExternalScope::OpenId, ExternalScope::GmailReadonly],
            state: GrantState::Active,
            granted_at_ms: 10,
            revoked_at_ms: None,
            version: 1,
        };
        grant.validate_m6_read_only().unwrap();
        grant.scopes.push(ExternalScope::GmailSend);
        assert_eq!(
            grant.validate_m6_read_only(),
            Err(ExternalIdentityContractError::WriteScopeDenied)
        );
        grant.scopes.pop();
        grant.state = GrantState::Revoked;
        grant.revoked_at_ms = Some(20);
        grant.validate().unwrap();
        assert_eq!(
            grant.validate_m6_read_only(),
            Err(ExternalIdentityContractError::WriteScopeDenied)
        );
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
}
