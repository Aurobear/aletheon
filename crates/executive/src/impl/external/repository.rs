use crate::r#impl::goal::migrations;
use async_trait::async_trait;
use corpus::tools::google::oauth::GoogleBinding;
use fabric::{
    CapabilityGrant, ExternalIdentity, ExternalIdentityId, ExternalIdentityState, ExternalScope,
    GrantState, IdentityProvider, PrincipalId,
};
use rusqlite::{params, Connection, OptionalExtension, Row};
use std::fmt;
use std::path::Path;

pub struct ExternalIdentityRepository {
    db: Connection,
}

impl ExternalIdentityRepository {
    pub fn open(path: &Path) -> Result<Self, ExternalRepositoryError> {
        let db = Connection::open(path)?;
        migrations::run_migrations(&db)
            .map_err(|error| ExternalRepositoryError::Storage(error.to_string()))?;
        Ok(Self { db })
    }

    /// Bind only a profile returned by the credential-owning Google OAuth
    /// provider. The authenticated caller supplies `principal_id`; it is never
    /// accepted from a model or provider payload.
    pub fn bind_google(
        &self,
        principal_id: &PrincipalId,
        binding: GoogleBinding,
        alias: Option<String>,
        now_ms: i64,
    ) -> Result<(ExternalIdentity, CapabilityGrant), ExternalRepositoryError> {
        let identity = ExternalIdentity {
            id: binding.identity_id,
            provider: IdentityProvider::Google,
            principal_id: principal_id.clone(),
            provider_subject: binding.provider_subject,
            email: binding.email,
            alias,
            state: ExternalIdentityState::Active,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
            version: 0,
        };
        identity.validate()?;
        let grant = CapabilityGrant {
            identity_id: identity.id,
            scopes: binding.scopes,
            state: GrantState::Active,
            granted_at_ms: now_ms,
            revoked_at_ms: None,
            version: 0,
        };
        grant.validate_m6_read_only()?;
        let scopes = serde_json::to_string(&grant.scopes)?;
        let tx = self.db.unchecked_transaction()?;
        let inserted = tx.execute(
            "INSERT OR IGNORE INTO external_identities (
                identity_id,provider,provider_subject,principal_id,email,alias,state,
                created_at_ms,updated_at_ms,version
             ) VALUES (?1,'google',?2,?3,?4,?5,'active',?6,?6,0)",
            params![
                identity.id.0.to_string(),
                identity.provider_subject,
                identity.principal_id.0,
                identity.email,
                identity.alias,
                now_ms,
            ],
        )?;
        if inserted != 1 {
            return Err(ExternalRepositoryError::DuplicateBinding);
        }
        tx.execute(
            "INSERT INTO capability_grants
             (identity_id,scopes_json,state,granted_at_ms,version)
             VALUES (?1,?2,'active',?3,0)",
            params![identity.id.0.to_string(), scopes, now_ms],
        )?;
        append_event(
            &tx,
            identity.id,
            "bound",
            &serde_json::json!({"scopes": grant.scopes}),
            now_ms,
        )?;
        tx.commit()?;
        Ok((identity, grant))
    }

    pub fn get(
        &self,
        principal_id: &PrincipalId,
        identity_id: ExternalIdentityId,
    ) -> Result<Option<(ExternalIdentity, CapabilityGrant)>, ExternalRepositoryError> {
        self.db
            .query_row(
                "SELECT i.identity_id,i.provider,i.principal_id,i.provider_subject,i.email,
                        i.alias,i.state,i.created_at_ms,i.updated_at_ms,i.version,
                        g.scopes_json,g.state,g.granted_at_ms,g.revoked_at_ms,g.version
                 FROM external_identities i JOIN capability_grants g USING(identity_id)
                 WHERE i.identity_id=?1 AND i.principal_id=?2",
                params![identity_id.0.to_string(), principal_id.0],
                decode_binding,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn list(
        &self,
        principal_id: &PrincipalId,
    ) -> Result<Vec<(ExternalIdentity, CapabilityGrant)>, ExternalRepositoryError> {
        let mut statement = self.db.prepare(
            "SELECT i.identity_id,i.provider,i.principal_id,i.provider_subject,i.email,
                    i.alias,i.state,i.created_at_ms,i.updated_at_ms,i.version,
                    g.scopes_json,g.state,g.granted_at_ms,g.revoked_at_ms,g.version
             FROM external_identities i JOIN capability_grants g USING(identity_id)
             WHERE i.principal_id=?1 ORDER BY i.created_at_ms,i.identity_id LIMIT 100",
        )?;
        let bindings = statement
            .query_map(params![principal_id.0], decode_binding)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(ExternalRepositoryError::from)?;
        Ok(bindings)
    }

    pub fn list_active(
        &self,
    ) -> Result<Vec<(ExternalIdentity, CapabilityGrant)>, ExternalRepositoryError> {
        let mut statement = self.db.prepare(
            "SELECT i.identity_id,i.provider,i.principal_id,i.provider_subject,i.email,
                    i.alias,i.state,i.created_at_ms,i.updated_at_ms,i.version,
                    g.scopes_json,g.state,g.granted_at_ms,g.revoked_at_ms,g.version
             FROM external_identities i JOIN capability_grants g USING(identity_id)
             WHERE i.state='active' AND g.state='active'
             ORDER BY i.created_at_ms,i.identity_id LIMIT 1000",
        )?;
        let bindings = statement
            .query_map([], decode_binding)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(ExternalRepositoryError::from)?;
        Ok(bindings)
    }

    pub fn has_active_scope(&self, scope: ExternalScope) -> Result<bool, ExternalRepositoryError> {
        let mut statement = self.db.prepare(
            "SELECT g.scopes_json FROM capability_grants g
             JOIN external_identities i USING(identity_id)
             WHERE g.state='active' AND i.state='active'",
        )?;
        for scopes_json in statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?
        {
            let scopes: Vec<ExternalScope> = serde_json::from_str(&scopes_json)?;
            if scopes.contains(&scope) && scopes.iter().all(|candidate| !candidate.is_write()) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub fn resolve_account(
        &self,
        principal_id: &PrincipalId,
        account_reference: &str,
    ) -> Result<Option<ExternalIdentityId>, ExternalRepositoryError> {
        if let Ok(uuid) = uuid::Uuid::parse_str(account_reference) {
            let id = ExternalIdentityId(uuid);
            return Ok(self.get(principal_id, id)?.map(|_| id));
        }
        let mut statement = self.db.prepare(
            "SELECT identity_id FROM external_identities
             WHERE principal_id=?1 AND alias=?2 AND state='active'
             ORDER BY identity_id LIMIT 2",
        )?;
        let ids = statement
            .query_map(params![principal_id.0, account_reference], |row| {
                let value: String = row.get(0)?;
                parse_uuid(&value, 0).map(ExternalIdentityId)
            })?
            .collect::<Result<Vec<_>, _>>()?;
        match ids.as_slice() {
            [] => Ok(None),
            [id] => Ok(Some(*id)),
            _ => Err(ExternalRepositoryError::AmbiguousAccount),
        }
    }

    pub fn update_grant(
        &self,
        principal_id: &PrincipalId,
        identity_id: ExternalIdentityId,
        expected_version: u64,
        scopes: Vec<ExternalScope>,
        now_ms: i64,
    ) -> Result<CapabilityGrant, ExternalRepositoryError> {
        let (_, current) = self
            .get(principal_id, identity_id)?
            .ok_or(ExternalRepositoryError::NotFound)?;
        if current.version != expected_version {
            return Err(ExternalRepositoryError::VersionConflict {
                expected: expected_version,
                actual: current.version,
            });
        }
        let next = CapabilityGrant {
            identity_id,
            scopes,
            state: GrantState::Active,
            granted_at_ms: current.granted_at_ms,
            revoked_at_ms: None,
            version: current.version + 1,
        };
        next.validate_m6_read_only()?;
        let tx = self.db.unchecked_transaction()?;
        let changed = tx.execute(
            "UPDATE capability_grants SET scopes_json=?1,version=?2
             WHERE identity_id=?3 AND version=?4 AND state='active'",
            params![
                serde_json::to_string(&next.scopes)?,
                next.version,
                identity_id.0.to_string(),
                current.version
            ],
        )?;
        if changed != 1 {
            return Err(ExternalRepositoryError::VersionConflict {
                expected: current.version,
                actual: self
                    .get(principal_id, identity_id)?
                    .map(|(_, grant)| grant.version)
                    .unwrap_or(0),
            });
        }
        tx.execute(
            "UPDATE external_identities SET updated_at_ms=?1,version=version+1
             WHERE identity_id=?2 AND principal_id=?3",
            params![now_ms, identity_id.0.to_string(), principal_id.0],
        )?;
        append_event(
            &tx,
            identity_id,
            "grant_updated",
            &serde_json::json!({"scopes": next.scopes}),
            now_ms,
        )?;
        tx.commit()?;
        Ok(next)
    }

    pub fn revoke_local(
        &self,
        principal_id: &PrincipalId,
        identity_id: ExternalIdentityId,
        expected_version: u64,
        now_ms: i64,
    ) -> Result<(ExternalIdentity, CapabilityGrant), ExternalRepositoryError> {
        let (identity, grant) = self
            .get(principal_id, identity_id)?
            .ok_or(ExternalRepositoryError::NotFound)?;
        if identity.version != expected_version {
            return Err(ExternalRepositoryError::VersionConflict {
                expected: expected_version,
                actual: identity.version,
            });
        }
        if identity.state == ExternalIdentityState::Revoked {
            return Ok((identity, grant));
        }
        let tx = self.db.unchecked_transaction()?;
        let changed = tx.execute(
            "UPDATE external_identities SET state='revoked',updated_at_ms=?1,version=version+1
             WHERE identity_id=?2 AND principal_id=?3 AND version=?4 AND state='active'",
            params![
                now_ms,
                identity_id.0.to_string(),
                principal_id.0,
                expected_version
            ],
        )?;
        if changed != 1 {
            return Err(ExternalRepositoryError::VersionConflict {
                expected: expected_version,
                actual: identity.version,
            });
        }
        tx.execute(
            "UPDATE capability_grants SET state='revoked',revoked_at_ms=?1,version=version+1
             WHERE identity_id=?2 AND state='active'",
            params![now_ms, identity_id.0.to_string()],
        )?;
        append_event(
            &tx,
            identity_id,
            "revoked",
            &serde_json::json!({"reason":"local_revocation"}),
            now_ms,
        )?;
        tx.commit()?;
        self.get(principal_id, identity_id)?
            .ok_or(ExternalRepositoryError::NotFound)
    }

    pub async fn revoke_with(
        &self,
        principal_id: &PrincipalId,
        identity_id: ExternalIdentityId,
        expected_version: u64,
        now_ms: i64,
        revoker: &dyn ExternalCredentialRevoker,
    ) -> Result<ExternalRevocationOutcome, ExternalRepositoryError> {
        let (identity, grant) =
            self.revoke_local(principal_id, identity_id, expected_version, now_ms)?;
        let provider_revocation_error = revoker
            .revoke_credentials(identity_id)
            .await
            .err()
            .map(|_| "provider_revocation_failed".to_owned());
        Ok(ExternalRevocationOutcome {
            identity,
            grant,
            provider_revocation_error,
        })
    }

    #[cfg(test)]
    fn event_count(&self, identity_id: ExternalIdentityId) -> i64 {
        self.db
            .query_row(
                "SELECT COUNT(*) FROM external_identity_events WHERE identity_id=?1",
                params![identity_id.0.to_string()],
                |row| row.get(0),
            )
            .unwrap()
    }
}

#[async_trait]
pub trait ExternalCredentialRevoker: Send + Sync {
    /// Delete local vault credentials first, then best-effort provider revoke.
    async fn revoke_credentials(&self, identity_id: ExternalIdentityId) -> anyhow::Result<()>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalRevocationOutcome {
    pub identity: ExternalIdentity,
    pub grant: CapabilityGrant,
    pub provider_revocation_error: Option<String>,
}

#[derive(Debug)]
pub enum ExternalRepositoryError {
    Storage(String),
    Contract(String),
    DuplicateBinding,
    NotFound,
    AmbiguousAccount,
    VersionConflict { expected: u64, actual: u64 },
}

impl fmt::Display for ExternalRepositoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Storage(_) => f.write_str("external identity storage failed"),
            Self::Contract(message) => write!(f, "external identity contract failed: {message}"),
            Self::DuplicateBinding => f.write_str("external account is already bound"),
            Self::NotFound => f.write_str("external account not found"),
            Self::AmbiguousAccount => f.write_str("external account alias is ambiguous"),
            Self::VersionConflict { expected, actual } => {
                write!(
                    f,
                    "external account version conflict: expected {expected}, actual {actual}"
                )
            }
        }
    }
}

impl std::error::Error for ExternalRepositoryError {}

impl From<rusqlite::Error> for ExternalRepositoryError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Storage(error.to_string())
    }
}
impl From<serde_json::Error> for ExternalRepositoryError {
    fn from(error: serde_json::Error) -> Self {
        Self::Storage(error.to_string())
    }
}
impl From<fabric::external_identity::ExternalIdentityContractError> for ExternalRepositoryError {
    fn from(error: fabric::external_identity::ExternalIdentityContractError) -> Self {
        Self::Contract(error.to_string())
    }
}

fn decode_binding(row: &Row<'_>) -> rusqlite::Result<(ExternalIdentity, CapabilityGrant)> {
    let id: String = row.get(0)?;
    let identity_id = ExternalIdentityId(parse_uuid(&id, 0)?);
    let provider: String = row.get(1)?;
    if provider != "google" {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let identity = ExternalIdentity {
        id: identity_id,
        provider: IdentityProvider::Google,
        principal_id: PrincipalId(row.get(2)?),
        provider_subject: row.get(3)?,
        email: row.get(4)?,
        alias: row.get(5)?,
        state: match row.get::<_, String>(6)?.as_str() {
            "active" => ExternalIdentityState::Active,
            "revoked" => ExternalIdentityState::Revoked,
            _ => return Err(rusqlite::Error::InvalidQuery),
        },
        created_at_ms: row.get(7)?,
        updated_at_ms: row.get(8)?,
        version: row.get(9)?,
    };
    let scopes_json: String = row.get(10)?;
    let grant = CapabilityGrant {
        identity_id,
        scopes: serde_json::from_str(&scopes_json).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(10, rusqlite::types::Type::Text, error.into())
        })?,
        state: match row.get::<_, String>(11)?.as_str() {
            "active" => GrantState::Active,
            "revoked" => GrantState::Revoked,
            _ => return Err(rusqlite::Error::InvalidQuery),
        },
        granted_at_ms: row.get(12)?,
        revoked_at_ms: row.get(13)?,
        version: row.get(14)?,
    };
    Ok((identity, grant))
}

fn parse_uuid(value: &str, column: usize) -> rusqlite::Result<uuid::Uuid> {
    uuid::Uuid::parse_str(value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(column, rusqlite::types::Type::Text, error.into())
    })
}

fn append_event(
    tx: &rusqlite::Transaction<'_>,
    identity_id: ExternalIdentityId,
    event_type: &str,
    payload: &serde_json::Value,
    now_ms: i64,
) -> Result<(), ExternalRepositoryError> {
    let payload = serde_json::to_string(payload)?;
    if payload.len() > 16 * 1024 {
        return Err(ExternalRepositoryError::Contract(
            "event payload exceeds bound".into(),
        ));
    }
    tx.execute(
        "INSERT INTO external_identity_events
         (identity_id,event_type,payload_json,created_at_ms) VALUES (?1,?2,?3,?4)",
        params![identity_id.0.to_string(), event_type, payload, now_ms],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn binding(subject: &str, email: &str, scopes: Vec<ExternalScope>) -> GoogleBinding {
        GoogleBinding {
            identity_id: ExternalIdentityId::new(),
            provider_subject: subject.into(),
            email: email.into(),
            scopes,
        }
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        path: std::path::PathBuf,
        repo: ExternalIdentityRepository,
        owner: PrincipalId,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("objectives.db");
            Self {
                repo: ExternalIdentityRepository::open(&path).unwrap(),
                _dir: dir,
                path,
                owner: PrincipalId("owner".into()),
            }
        }

        fn bind(&self, subject: &str, email: &str) -> (ExternalIdentity, CapabilityGrant) {
            self.repo
                .bind_google(
                    &self.owner,
                    binding(
                        subject,
                        email,
                        vec![ExternalScope::OpenId, ExternalScope::GmailReadonly],
                    ),
                    None,
                    10,
                )
                .unwrap()
        }
    }

    #[test]
    fn multiple_accounts_duplicate_binding_and_restart_are_durable() {
        let f = Fixture::new();
        let first = f.bind("subject-1", "one@example.com");
        let second = f.bind("subject-2", "two@example.com");
        assert_ne!(first.0.id, second.0.id);
        let duplicate = f.repo.bind_google(
            &f.owner,
            binding("subject-1", "one@example.com", vec![ExternalScope::OpenId]),
            None,
            20,
        );
        assert!(matches!(
            duplicate,
            Err(ExternalRepositoryError::DuplicateBinding)
        ));
        drop(f.repo);
        let reopened = ExternalIdentityRepository::open(&f.path).unwrap();
        assert_eq!(reopened.list(&f.owner).unwrap().len(), 2);
        assert_eq!(reopened.event_count(first.0.id), 1);
    }

    #[test]
    fn principal_isolation_and_write_scope_rejection_fail_closed() {
        let f = Fixture::new();
        let (identity, _) = f.bind("subject", "owner@example.com");
        let attacker = PrincipalId("attacker".into());
        assert!(f.repo.get(&attacker, identity.id).unwrap().is_none());
        assert!(f.repo.list(&attacker).unwrap().is_empty());
        let rejected = f.repo.bind_google(
            &attacker,
            binding(
                "attacker-subject",
                "attacker@example.com",
                vec![ExternalScope::GmailSend],
            ),
            None,
            10,
        );
        assert!(matches!(
            rejected,
            Err(ExternalRepositoryError::Contract(_))
        ));
    }

    #[test]
    fn reduced_grants_use_optimistic_versions() {
        let f = Fixture::new();
        let (identity, grant) = f.bind("subject", "owner@example.com");
        let reduced = f
            .repo
            .update_grant(
                &f.owner,
                identity.id,
                grant.version,
                vec![ExternalScope::OpenId],
                20,
            )
            .unwrap();
        assert_eq!(reduced.scopes, vec![ExternalScope::OpenId]);
        assert!(matches!(
            f.repo.update_grant(
                &f.owner,
                identity.id,
                grant.version,
                vec![ExternalScope::OpenId],
                30
            ),
            Err(ExternalRepositoryError::VersionConflict { .. })
        ));
    }

    struct Revoker {
        calls: AtomicUsize,
        fail: bool,
    }
    #[async_trait]
    impl ExternalCredentialRevoker for Revoker {
        async fn revoke_credentials(&self, _: ExternalIdentityId) -> anyhow::Result<()> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            anyhow::ensure!(!self.fail, "provider response contained access-secret");
            Ok(())
        }
    }

    #[tokio::test]
    async fn local_revocation_survives_missing_or_failed_provider_revocation() {
        let f = Fixture::new();
        let (identity, _) = f.bind("subject", "owner@example.com");
        let revoker = Revoker {
            calls: AtomicUsize::new(0),
            fail: true,
        };
        let outcome = f
            .repo
            .revoke_with(&f.owner, identity.id, identity.version, 20, &revoker)
            .await
            .unwrap();
        assert_eq!(outcome.identity.state, ExternalIdentityState::Revoked);
        assert_eq!(outcome.grant.state, GrantState::Revoked);
        assert_eq!(
            outcome.provider_revocation_error.as_deref(),
            Some("provider_revocation_failed")
        );
        assert!(!format!("{outcome:?}").contains("access-secret"));
        drop(f.repo);
        let reopened = ExternalIdentityRepository::open(&f.path).unwrap();
        let persisted = reopened.get(&f.owner, identity.id).unwrap().unwrap();
        assert_eq!(persisted.0.state, ExternalIdentityState::Revoked);
        assert_eq!(persisted.1.state, GrantState::Revoked);
    }
}
