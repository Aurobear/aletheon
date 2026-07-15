//! AES-256-GCM credential persistence with strict Unix file protections.

use crate::tools::mcp::token_store::{TokenEntry, TokenKey, TokenPersistence};
use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

const MAGIC: &str = "ALETHEON-CREDENTIAL-VAULT";
const VERSION: u8 = 1;
const KEY_BYTES: usize = 32;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct VaultEnvelope {
    magic: String,
    version: u8,
    nonce: [u8; 12],
    ciphertext: Vec<u8>,
}

/// Encrypted persistence backend. The key is loaded from a root-owned 0600
/// file and never exposed through Debug, serialization, or public accessors.
pub struct CredentialVault {
    path: PathBuf,
    key: [u8; KEY_BYTES],
}

impl std::fmt::Debug for CredentialVault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CredentialVault")
            .field("path", &self.path)
            .field("key", &"[REDACTED]")
            .finish()
    }
}

impl Drop for CredentialVault {
    fn drop(&mut self) {
        self.key.fill(0);
    }
}

impl CredentialVault {
    pub fn open(path: PathBuf, key_path: &Path) -> Result<Self> {
        let key = load_master_key(key_path, true)?;
        let vault = Self { path, key };
        if vault.path.exists() {
            require_secure_file(&vault.path, false)?;
            vault.load_all().context("validating credential vault")?;
        }
        Ok(vault)
    }

    /// Create a new root-owned 0600 key file from operating-system entropy.
    #[cfg(unix)]
    pub fn create_master_key(path: &Path) -> Result<()> {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        anyhow::ensure!(
            unsafe { libc::geteuid() } == 0,
            "master key creation requires root"
        );
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let key = Aes256Gcm::generate_key(&mut OsRng);
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(path)
            .context("creating credential master key")?;
        file.write_all(key.as_slice())?;
        file.sync_all()?;
        anyhow::ensure!(
            file.metadata()?.uid() == 0,
            "credential master key is not root-owned"
        );
        sync_parent(path)?;
        Ok(())
    }

    #[cfg(not(unix))]
    pub fn create_master_key(_: &Path) -> Result<()> {
        anyhow::bail!("credential vault ACL validation is unavailable on this platform")
    }

    #[cfg(test)]
    fn open_for_test(path: PathBuf, key_path: &Path) -> Result<Self> {
        Ok(Self {
            path,
            key: load_master_key(key_path, false)?,
        })
    }

    fn aad() -> Vec<u8> {
        [MAGIC.as_bytes(), &[VERSION]].concat()
    }

    fn decrypt(&self, envelope: VaultEnvelope) -> Result<HashMap<TokenKey, TokenEntry>> {
        anyhow::ensure!(envelope.magic == MAGIC, "credential vault magic mismatch");
        anyhow::ensure!(
            envelope.version == VERSION,
            "unsupported credential vault version"
        );
        let cipher =
            Aes256Gcm::new_from_slice(&self.key).context("initializing credential vault")?;
        let plaintext = cipher
            .decrypt(
                Nonce::from_slice(&envelope.nonce),
                Payload {
                    msg: &envelope.ciphertext,
                    aad: &Self::aad(),
                },
            )
            .map_err(|_| anyhow::anyhow!("credential vault authentication failed"))?;
        serde_json::from_slice(&plaintext).context("decoding credential vault payload")
    }

    fn encrypt(&self, entries: &HashMap<TokenKey, TokenEntry>) -> Result<Vec<u8>> {
        let plaintext = serde_json::to_vec(entries).context("encoding credential vault payload")?;
        let cipher =
            Aes256Gcm::new_from_slice(&self.key).context("initializing credential vault")?;
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ciphertext = cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: &plaintext,
                    aad: &Self::aad(),
                },
            )
            .map_err(|_| anyhow::anyhow!("credential vault encryption failed"))?;
        let envelope = VaultEnvelope {
            magic: MAGIC.into(),
            version: VERSION,
            nonce: nonce.into(),
            ciphertext,
        };
        serde_json::to_vec(&envelope).context("encoding credential vault envelope")
    }

    #[cfg(unix)]
    fn atomic_write(&self, bytes: &[u8]) -> Result<()> {
        use std::os::unix::fs::OpenOptionsExt;
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        if self.path.exists() {
            require_secure_file(&self.path, false)?;
        }
        let name = self
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("credentials.vault");
        let temp = self
            .path
            .with_file_name(format!(".{name}.{}.tmp", uuid::Uuid::new_v4()));
        let result = (|| -> Result<()> {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .open(&temp)
                .context("creating temporary credential vault")?;
            file.write_all(bytes).context("writing credential vault")?;
            file.sync_all().context("syncing credential vault")?;
            fs::rename(&temp, &self.path).context("replacing credential vault")?;
            sync_parent(&self.path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(temp);
        }
        result
    }

    #[cfg(not(unix))]
    fn atomic_write(&self, _: &[u8]) -> Result<()> {
        anyhow::bail!("credential vault ACL validation is unavailable on this platform")
    }
}

impl TokenPersistence for CredentialVault {
    fn load_all(&self) -> Result<HashMap<TokenKey, TokenEntry>> {
        if !self.path.exists() {
            return Ok(HashMap::new());
        }
        require_secure_file(&self.path, false)?;
        let bytes = fs::read(&self.path).context("reading credential vault")?;
        let envelope: VaultEnvelope =
            serde_json::from_slice(&bytes).context("decoding credential vault envelope")?;
        self.decrypt(envelope)
    }

    fn replace_all(&self, entries: &HashMap<TokenKey, TokenEntry>) -> Result<()> {
        self.atomic_write(&self.encrypt(entries)?)
    }
}

#[cfg(unix)]
fn load_master_key(path: &Path, require_root: bool) -> Result<[u8; KEY_BYTES]> {
    require_secure_file(path, require_root)?;
    let bytes = fs::read(path).context("reading credential master key")?;
    anyhow::ensure!(
        bytes.len() == KEY_BYTES,
        "credential master key has invalid length"
    );
    let mut key = [0_u8; KEY_BYTES];
    key.copy_from_slice(&bytes);
    Ok(key)
}

#[cfg(not(unix))]
fn load_master_key(_: &Path, _: bool) -> Result<[u8; KEY_BYTES]> {
    anyhow::bail!("credential vault ACL validation is unavailable on this platform")
}

#[cfg(unix)]
fn require_secure_file(path: &Path, require_root: bool) -> Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let metadata = fs::symlink_metadata(path).context("reading credential file metadata")?;
    anyhow::ensure!(
        metadata.file_type().is_file(),
        "credential path is not a regular file"
    );
    let mode = metadata.permissions().mode() & 0o777;
    let credential_copy = require_root
        && std::env::var_os("CREDENTIALS_DIRECTORY")
            .is_some_and(|directory| path.starts_with(PathBuf::from(directory)));
    if credential_copy {
        anyhow::ensure!(
            matches!(mode, 0o400 | 0o440 | 0o600),
            "systemd credential copy mode must be 0400, 0440, or 0600"
        );
        anyhow::ensure!(
            matches!(metadata.uid(), 0) || metadata.uid() == unsafe { libc::geteuid() },
            "systemd credential copy is not owned by root or the service user"
        );
    } else {
        anyhow::ensure!(mode == 0o600, "credential file mode must be 0600");
    }
    if require_root {
        anyhow::ensure!(
            credential_copy || metadata.uid() == 0,
            "credential master key must be root-owned"
        );
    }
    Ok(())
}

#[cfg(not(unix))]
fn require_secure_file(_: &Path, _: bool) -> Result<()> {
    anyhow::bail!("credential vault ACL validation is unavailable on this platform")
}

fn sync_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        OpenOptions::new().read(true).open(parent)?.sync_all()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::mcp::token_store::{JsonTokenPersistence, TokenStore};
    use fabric::{ExternalIdentityId, IdentityProvider};

    #[cfg(unix)]
    fn fixture() -> (tempfile::TempDir, CredentialVault, TokenKey, TokenEntry) {
        use std::os::unix::fs::OpenOptionsExt;
        let dir = tempfile::tempdir().unwrap();
        let key_path = dir.path().join("key");
        let mut key = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&key_path)
            .unwrap();
        key.write_all(&[7_u8; KEY_BYTES]).unwrap();
        key.sync_all().unwrap();
        let vault =
            CredentialVault::open_for_test(dir.path().join("tokens.vault"), &key_path).unwrap();
        let token_key = TokenKey::external(IdentityProvider::Google, ExternalIdentityId::new());
        let entry = TokenEntry {
            access_token: "access-token-secret".into(),
            refresh_token: Some("refresh-token-secret".into()),
            expires_at: 99,
            token_type: "Bearer".into(),
            scopes: vec!["gmail.readonly".into(), "account-email@example.com".into()],
        };
        (dir, vault, token_key, entry)
    }

    #[cfg(unix)]
    #[test]
    fn ciphertext_hides_tokens_email_and_serialized_payload() {
        let (_dir, vault, key, entry) = fixture();
        vault.write(key.clone(), entry.clone()).unwrap();
        let bytes = fs::read(&vault.path).unwrap();
        let rendered = String::from_utf8_lossy(&bytes);
        for secret in [
            "access-token-secret",
            "refresh-token-secret",
            "account-email@example.com",
            "access_token",
        ] {
            assert!(!rendered.contains(secret));
        }
        assert_eq!(vault.read(&key).unwrap(), Some(entry));
    }

    #[cfg(unix)]
    #[test]
    fn fresh_nonce_changes_ciphertext_for_identical_entries() {
        let (_dir, vault, key, entry) = fixture();
        vault.write(key, entry).unwrap();
        let first = fs::read(&vault.path).unwrap();
        let entries = vault.load_all().unwrap();
        vault.replace_all(&entries).unwrap();
        let second = fs::read(&vault.path).unwrap();
        assert_ne!(first, second);
    }

    #[cfg(unix)]
    #[test]
    fn wrong_key_tampering_truncation_and_version_are_rejected() {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let (dir, vault, key, entry) = fixture();
        vault.write(key, entry).unwrap();
        let original = fs::read(&vault.path).unwrap();

        let wrong_key_path = dir.path().join("wrong-key");
        let mut wrong = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&wrong_key_path)
            .unwrap();
        wrong.write_all(&[8_u8; KEY_BYTES]).unwrap();
        assert!(
            CredentialVault::open_for_test(vault.path.clone(), &wrong_key_path)
                .unwrap()
                .load_all()
                .is_err()
        );

        let mut envelope: VaultEnvelope = serde_json::from_slice(&original).unwrap();
        envelope.ciphertext[0] ^= 1;
        fs::write(&vault.path, serde_json::to_vec(&envelope).unwrap()).unwrap();
        fs::set_permissions(&vault.path, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(vault.load_all().is_err());

        fs::write(&vault.path, &original[..original.len() / 2]).unwrap();
        assert!(vault.load_all().is_err());

        let mut envelope: VaultEnvelope = serde_json::from_slice(&original).unwrap();
        envelope.version += 1;
        fs::write(&vault.path, serde_json::to_vec(&envelope).unwrap()).unwrap();
        assert!(vault.load_all().is_err());
    }

    #[cfg(unix)]
    #[test]
    fn mode_checks_fail_closed() {
        use std::os::unix::fs::PermissionsExt;
        let (_dir, vault, key, entry) = fixture();
        vault.write(key, entry).unwrap();
        fs::set_permissions(&vault.path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(vault.load_all().is_err());
    }

    #[cfg(unix)]
    #[test]
    fn explicit_legacy_migration_verifies_then_securely_removes_plaintext() {
        let (dir, vault, key, entry) = fixture();
        let legacy_path = dir.path().join("legacy.json");
        let mut legacy = TokenStore::new(legacy_path.clone()).unwrap();
        legacy.set_key(key.clone(), entry.clone());
        legacy.save().unwrap();
        assert!(String::from_utf8_lossy(&fs::read(&legacy_path).unwrap())
            .contains("access-token-secret"));
        assert_eq!(legacy.migrate_to(&vault).unwrap(), 1);
        assert!(!legacy_path.exists());
        assert_eq!(vault.read(&key).unwrap(), Some(entry));
    }

    #[cfg(unix)]
    #[test]
    fn failed_migration_keeps_plaintext_source() {
        let (dir, _vault, key, entry) = fixture();
        let legacy_path = dir.path().join("legacy.json");
        let mut legacy = TokenStore::new(legacy_path.clone()).unwrap();
        legacy.set_key(key, entry);
        legacy.save().unwrap();
        let bad_target = JsonTokenPersistence::new(dir.path().join("missing/../bad\0path"));
        assert!(legacy.migrate_to(&bad_target).is_err());
        assert!(legacy_path.exists());
        assert!(String::from_utf8_lossy(&fs::read(legacy_path).unwrap())
            .contains("access-token-secret"));
    }
}
