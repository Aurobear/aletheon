//! OAuth token persistence boundary.
//!
//! `TokenStore` remains the compatibility facade used by MCP OAuth. Provider
//! integrations use `TokenKey` so credentials are keyed by provider and
//! external identity rather than by an untrusted account label.

use anyhow::{Context, Result};
use fabric::{ExternalIdentityId, IdentityProvider};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TokenEntry {
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// Unix epoch seconds when the access token expires.
    pub expires_at: u64,
    pub token_type: String,
    pub scopes: Vec<String>,
}

impl fmt::Debug for TokenEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TokenEntry")
            .field("access_token", &"[REDACTED]")
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("expires_at", &self.expires_at)
            .field("token_type", &self.token_type)
            .field("scopes", &self.scopes)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TokenKey(String);

impl TokenKey {
    pub fn external(provider: IdentityProvider, identity_id: ExternalIdentityId) -> Self {
        Self(format!("external:{provider}:{identity_id}"))
    }

    pub fn legacy_mcp(server_id: &str) -> Self {
        Self(server_id.to_owned())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Whole-entry persistence operations. Implementations must make `replace_all`
/// atomic; encrypted persistence is supplied by the credential vault in M6 T3.
pub trait TokenPersistence: Send + Sync {
    fn load_all(&self) -> Result<HashMap<TokenKey, TokenEntry>>;
    fn replace_all(&self, entries: &HashMap<TokenKey, TokenEntry>) -> Result<()>;

    /// Remove legacy persistence after a verified one-shot migration.
    fn finalize_migration(&self) -> Result<()> {
        Ok(())
    }

    fn read(&self, key: &TokenKey) -> Result<Option<TokenEntry>> {
        Ok(self.load_all()?.remove(key))
    }

    fn write(&self, key: TokenKey, entry: TokenEntry) -> Result<()> {
        let mut entries = self.load_all()?;
        entries.insert(key, entry);
        self.replace_all(&entries)
    }

    fn delete(&self, key: &TokenKey) -> Result<Option<TokenEntry>> {
        let mut entries = self.load_all()?;
        let removed = entries.remove(key);
        self.replace_all(&entries)?;
        Ok(removed)
    }
}

#[derive(Debug)]
pub struct JsonTokenPersistence {
    path: PathBuf,
}

impl JsonTokenPersistence {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn atomic_write(&self, bytes: &[u8]) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating token directory {}", parent.display()))?;
        }
        let tmp = sibling_temp_path(&self.path);
        let write_result = (|| -> Result<()> {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&tmp)
                .with_context(|| format!("creating temporary token store {}", tmp.display()))?;
            file.write_all(bytes)
                .context("writing temporary token store")?;
            file.sync_all().context("syncing temporary token store")?;
            fs::rename(&tmp, &self.path)
                .with_context(|| format!("replacing token store {}", self.path.display()))?;
            if let Some(parent) = self.path.parent() {
                OpenOptions::new()
                    .read(true)
                    .open(parent)
                    .and_then(|directory| directory.sync_all())
                    .with_context(|| format!("syncing token directory {}", parent.display()))?;
            }
            Ok(())
        })();
        if write_result.is_err() {
            let _ = fs::remove_file(tmp);
        }
        write_result
    }
}

impl TokenPersistence for JsonTokenPersistence {
    fn load_all(&self) -> Result<HashMap<TokenKey, TokenEntry>> {
        if !self.path.exists() {
            return Ok(HashMap::new());
        }
        let data = fs::read(&self.path)
            .with_context(|| format!("reading token store {}", self.path.display()))?;
        serde_json::from_slice(&data)
            .with_context(|| format!("parsing token store {}", self.path.display()))
    }

    fn replace_all(&self, entries: &HashMap<TokenKey, TokenEntry>) -> Result<()> {
        let json = serde_json::to_vec_pretty(entries).context("serializing token store")?;
        self.atomic_write(&json)
    }

    fn finalize_migration(&self) -> Result<()> {
        if !self.path.exists() {
            return Ok(());
        }
        let len = fs::metadata(&self.path)?.len();
        let mut file = OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&self.path)?;
        let zeros = vec![0_u8; usize::try_from(len).unwrap_or(0).min(16 * 1024 * 1024)];
        file.write_all(&zeros)?;
        file.sync_all()?;
        drop(file);
        fs::remove_file(&self.path)?;
        if let Some(parent) = self.path.parent() {
            OpenOptions::new().read(true).open(parent)?.sync_all()?;
        }
        Ok(())
    }
}

pub struct TokenStore {
    persistence: Box<dyn TokenPersistence>,
    tokens: HashMap<TokenKey, TokenEntry>,
}

impl fmt::Debug for TokenStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TokenStore")
            .field("entry_count", &self.tokens.len())
            .finish()
    }
}

impl TokenStore {
    pub fn new(storage_path: PathBuf) -> Result<Self> {
        Self::from_persistence(Box::new(JsonTokenPersistence::new(storage_path)))
    }

    pub fn from_persistence(persistence: Box<dyn TokenPersistence>) -> Result<Self> {
        let tokens = persistence.load_all()?;
        Ok(Self {
            persistence,
            tokens,
        })
    }

    pub fn default_path() -> Result<PathBuf> {
        Ok(fabric::paths::credential_vault_path())
    }

    pub fn open_default() -> Result<Self> {
        let persistence = crate::security::credential_vault::CredentialVault::open(
            Self::default_path()?,
            &fabric::paths::credential_master_key_path(),
        )?;
        Self::from_persistence(Box::new(persistence))
    }

    /// Legacy plaintext path is exposed only for an explicit one-shot
    /// migration; it is never read by `open_default`.
    pub fn legacy_default_path() -> PathBuf {
        fabric::paths::mcp_tokens_path()
    }

    pub fn get(&self, server_id: &str) -> Option<&TokenEntry> {
        self.get_key(&TokenKey::legacy_mcp(server_id))
    }

    pub fn get_key(&self, key: &TokenKey) -> Option<&TokenEntry> {
        self.tokens.get(key)
    }

    pub fn set(&mut self, server_id: impl Into<String>, entry: TokenEntry) {
        let server_id = server_id.into();
        self.set_key(TokenKey::legacy_mcp(&server_id), entry);
    }

    pub fn set_key(&mut self, key: TokenKey, entry: TokenEntry) {
        self.tokens.insert(key, entry);
    }

    pub fn remove(&mut self, server_id: &str) -> Option<TokenEntry> {
        self.remove_key(&TokenKey::legacy_mcp(server_id))
    }

    pub fn remove_key(&mut self, key: &TokenKey) -> Option<TokenEntry> {
        self.tokens.remove(key)
    }

    pub fn save(&self) -> Result<()> {
        self.persistence.replace_all(&self.tokens)
    }

    /// Explicit one-shot migration. Source entries are deleted only after all
    /// target writes have been reread and matched byte-for-byte as entries.
    pub fn migrate_to(&self, target: &dyn TokenPersistence) -> Result<usize> {
        for (key, entry) in &self.tokens {
            target.write(key.clone(), entry.clone())?;
            let reread = target
                .read(key)?
                .with_context(|| format!("migration reread missing key {}", key.as_str()))?;
            anyhow::ensure!(&reread == entry, "migration reread mismatch");
        }
        for key in self.tokens.keys() {
            self.persistence.delete(key)?;
        }
        self.persistence.finalize_migration()?;
        Ok(self.tokens.len())
    }

    pub fn len(&self) -> usize {
        self.tokens.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }
}

fn sibling_temp_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("tokens");
    path.with_file_name(format!(".{file_name}.{}.tmp", uuid::Uuid::new_v4()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn entry(token: &str) -> TokenEntry {
        TokenEntry {
            access_token: token.into(),
            refresh_token: Some(format!("refresh-{token}")),
            expires_at: 42,
            token_type: "Bearer".into(),
            scopes: vec!["read".into()],
        }
    }

    #[test]
    fn persistence_contract_supports_whole_entry_crud_and_atomic_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tokens.json");
        let persistence = JsonTokenPersistence::new(path.clone());
        let key = TokenKey::legacy_mcp("server");
        assert_eq!(persistence.read(&key).unwrap(), None);
        persistence.write(key.clone(), entry("one")).unwrap();
        assert_eq!(persistence.read(&key).unwrap(), Some(entry("one")));
        persistence.write(key.clone(), entry("two")).unwrap();
        assert_eq!(persistence.read(&key).unwrap(), Some(entry("two")));
        assert_eq!(persistence.delete(&key).unwrap(), Some(entry("two")));
        assert_eq!(persistence.read(&key).unwrap(), None);
        assert!(!path.with_extension("tmp").exists());
    }

    #[test]
    fn external_keys_keep_multiple_accounts_isolated() {
        let dir = tempfile::tempdir().unwrap();
        let persistence = JsonTokenPersistence::new(dir.path().join("tokens.json"));
        let first = TokenKey::external(IdentityProvider::Google, ExternalIdentityId::new());
        let second = TokenKey::external(IdentityProvider::Google, ExternalIdentityId::new());
        persistence.write(first.clone(), entry("first")).unwrap();
        persistence.write(second.clone(), entry("second")).unwrap();
        assert_eq!(persistence.read(&first).unwrap(), Some(entry("first")));
        assert_eq!(persistence.read(&second).unwrap(), Some(entry("second")));
    }

    struct MemoryPersistence {
        entries: Mutex<HashMap<TokenKey, TokenEntry>>,
        fail_write: bool,
    }

    impl TokenPersistence for MemoryPersistence {
        fn load_all(&self) -> Result<HashMap<TokenKey, TokenEntry>> {
            Ok(self.entries.lock().unwrap().clone())
        }
        fn replace_all(&self, entries: &HashMap<TokenKey, TokenEntry>) -> Result<()> {
            anyhow::ensure!(!self.fail_write, "injected write failure");
            *self.entries.lock().unwrap() = entries.clone();
            Ok(())
        }
    }

    #[test]
    fn migration_preserves_source_on_failed_target_write() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("legacy.json");
        let mut source = TokenStore::new(source_path.clone()).unwrap();
        source.set("server", entry("secret"));
        source.save().unwrap();
        let target = MemoryPersistence {
            entries: Mutex::new(HashMap::new()),
            fail_write: true,
        };
        assert!(source.migrate_to(&target).is_err());
        assert!(TokenStore::new(source_path)
            .unwrap()
            .get("server")
            .is_some());
    }

    #[test]
    fn migration_rereads_target_before_deleting_source() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("legacy.json");
        let mut source = TokenStore::new(source_path.clone()).unwrap();
        source.set("server", entry("secret"));
        source.save().unwrap();
        let target = MemoryPersistence {
            entries: Mutex::new(HashMap::new()),
            fail_write: false,
        };
        assert_eq!(source.migrate_to(&target).unwrap(), 1);
        assert!(TokenStore::new(source_path).unwrap().is_empty());
        assert_eq!(
            target.read(&TokenKey::legacy_mcp("server")).unwrap(),
            Some(entry("secret"))
        );
    }

    #[test]
    fn token_entry_debug_is_redacted() {
        let rendered = format!("{:?}", entry("access-secret"));
        assert!(!rendered.contains("access-secret"));
        assert!(!rendered.contains("refresh-access-secret"));
    }
}
