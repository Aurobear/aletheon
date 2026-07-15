//! Centralized development and production path contracts for Aletheon.

use std::path::{Component, Path, PathBuf};

pub const PRODUCTION_STATE_ROOT: &str = "/var/lib/aletheon";
pub const PRODUCTION_CONFIG_ROOT: &str = "/etc/aletheon";
pub const PRODUCTION_RUNTIME_ROOT: &str = "/run/aletheon";
pub const PRODUCTION_CACHE_ROOT: &str = "/var/cache/aletheon";

/// Canonical runtime socket directory. Linux supplies `/var/run -> /run`
/// compatibility; application code always emits the canonical `/run` spelling.
pub const SOCKET_DIR: &str = PRODUCTION_RUNTIME_ROOT;
pub const SNAPSHOT_DIR: &str = "/var/lib/aletheon/state/snapshots";
pub const HOOKS_SYSTEM_DIR: &str = "/etc/aletheon/hooks";
pub const CGROUP_PREFIX: &str = "aletheon";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductionPaths {
    pub state_root: PathBuf,
    pub config_root: PathBuf,
    pub runtime_root: PathBuf,
    pub cache_root: PathBuf,
    pub state: PathBuf,
    pub goals: PathBuf,
    pub sessions: PathBuf,
    pub mnemosyne: PathBuf,
    pub artifacts: PathBuf,
    pub worktrees: PathBuf,
    pub audit: PathBuf,
    pub secret_root: PathBuf,
}

impl Default for ProductionPaths {
    fn default() -> Self {
        let state_root = PathBuf::from(PRODUCTION_STATE_ROOT);
        let config_root = PathBuf::from(PRODUCTION_CONFIG_ROOT);
        Self {
            state: state_root.join("state"),
            goals: state_root.join("goals"),
            sessions: state_root.join("sessions"),
            mnemosyne: state_root.join("mnemosyne"),
            artifacts: state_root.join("artifacts"),
            worktrees: state_root.join("worktrees"),
            audit: state_root.join("audit"),
            secret_root: config_root.join("credentials"),
            state_root,
            config_root,
            runtime_root: PathBuf::from(PRODUCTION_RUNTIME_ROOT),
            cache_root: PathBuf::from(PRODUCTION_CACHE_ROOT),
        }
    }
}

impl ProductionPaths {
    pub fn socket_path(&self) -> PathBuf {
        self.runtime_root.join("aletheon.sock")
    }

    /// Validate lexical containment and, when present, filesystem safety.
    /// Missing directories are accepted during pre-install validation unless
    /// `require_existing` is true.
    pub fn validate(&self, require_existing: bool) -> Result<(), PathContractError> {
        let roots = [
            (
                &self.state_root,
                Path::new(PRODUCTION_STATE_ROOT),
                "state_root",
            ),
            (
                &self.config_root,
                Path::new(PRODUCTION_CONFIG_ROOT),
                "config_root",
            ),
            (
                &self.runtime_root,
                Path::new(PRODUCTION_RUNTIME_ROOT),
                "runtime_root",
            ),
            (
                &self.cache_root,
                Path::new(PRODUCTION_CACHE_ROOT),
                "cache_root",
            ),
        ];
        for (path, approved, field) in roots {
            validate_absolute(path, field)?;
            if path != approved {
                return Err(PathContractError::OutsideApprovedRoot(field));
            }
            validate_existing_directory(path, field, require_existing, false)?;
        }
        for (path, field) in [
            (&self.state, "state"),
            (&self.goals, "goals"),
            (&self.sessions, "sessions"),
            (&self.mnemosyne, "mnemosyne"),
            (&self.artifacts, "artifacts"),
            (&self.worktrees, "worktrees"),
            (&self.audit, "audit"),
        ] {
            validate_child(path, &self.state_root, field)?;
            validate_existing_directory(path, field, require_existing, false)?;
        }
        validate_child(&self.secret_root, &self.config_root, "secret_root")?;
        validate_existing_directory(&self.secret_root, "secret_root", require_existing, true)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PathContractError {
    #[error("{0} must be an absolute normalized path")]
    InvalidPath(&'static str),
    #[error("{0} is outside its approved production root")]
    OutsideApprovedRoot(&'static str),
    #[error("{0} does not exist")]
    Missing(&'static str),
    #[error("{0} is not a directory")]
    NotDirectory(&'static str),
    #[error("{0} or one of its existing ancestors is symlinked")]
    Symlinked(&'static str),
    #[error("{0} is world-writable")]
    WorldWritable(&'static str),
    #[error("unable to inspect {0}")]
    Inspection(&'static str),
}

fn validate_child(path: &Path, root: &Path, field: &'static str) -> Result<(), PathContractError> {
    validate_absolute(path, field)?;
    if path == root || !path.starts_with(root) {
        return Err(PathContractError::OutsideApprovedRoot(field));
    }
    Ok(())
}

fn validate_absolute(path: &Path, field: &'static str) -> Result<(), PathContractError> {
    if !path.is_absolute()
        || path.to_string_lossy().contains('~')
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::CurDir | Component::Prefix(_)
            )
        })
    {
        return Err(PathContractError::InvalidPath(field));
    }
    Ok(())
}

fn validate_existing_directory(
    path: &Path,
    field: &'static str,
    require_existing: bool,
    reject_symlink_ancestors: bool,
) -> Result<(), PathContractError> {
    if reject_symlink_ancestors {
        reject_existing_symlinks(path, field)?;
    }
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && !require_existing => {
            return Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(PathContractError::Missing(field))
        }
        Err(_) => return Err(PathContractError::Inspection(field)),
    };
    if metadata.file_type().is_symlink() {
        return Err(PathContractError::Symlinked(field));
    }
    if !metadata.is_dir() {
        return Err(PathContractError::NotDirectory(field));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o002 != 0 {
            return Err(PathContractError::WorldWritable(field));
        }
    }
    Ok(())
}

fn reject_existing_symlinks(path: &Path, field: &'static str) -> Result<(), PathContractError> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(PathContractError::Symlinked(field))
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(_) => return Err(PathContractError::Inspection(field)),
        }
    }
    Ok(())
}

/// User config directory: ~/.aletheon/.
pub fn config_dir() -> PathBuf {
    home_dir().join(".aletheon")
}

pub fn socket_path() -> PathBuf {
    PathBuf::from(SOCKET_DIR).join("aletheon.sock")
}

pub fn xdg_config_dir() -> PathBuf {
    home_dir().join(".config").join("aletheon")
}

pub fn xdg_data_dir() -> PathBuf {
    home_dir().join(".local").join("share").join("aletheon")
}

pub fn user_hooks_dir() -> PathBuf {
    config_dir().join("hooks")
}

pub fn local_hooks_dir() -> PathBuf {
    PathBuf::from(".aletheon").join("hooks")
}

pub fn skills_dir() -> PathBuf {
    config_dir().join("skills")
}

pub fn agents_dir() -> PathBuf {
    config_dir().join("agents")
}

pub fn mcp_tokens_path() -> PathBuf {
    xdg_config_dir().join("mcp_tokens.json")
}

pub fn credential_vault_path() -> PathBuf {
    std::env::var_os("ALETHEON_CREDENTIAL_VAULT_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| xdg_config_dir().join("credentials.vault"))
}

pub fn credential_master_key_path() -> PathBuf {
    std::env::var_os("ALETHEON_CREDENTIAL_MASTER_KEY_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(PRODUCTION_CONFIG_ROOT)
                .join("credentials")
                .join("google-vault.key")
        })
}

pub fn config_file() -> PathBuf {
    config_dir().join("config.toml")
}

pub fn env_file() -> PathBuf {
    config_dir().join(".env")
}

fn home_dir() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_production_layout_uses_run_not_var_run() {
        let paths = ProductionPaths::default();
        assert_eq!(paths.runtime_root, Path::new("/run/aletheon"));
        assert_eq!(
            paths.socket_path(),
            Path::new("/run/aletheon/aletheon.sock")
        );
        assert_eq!(socket_path(), paths.socket_path());
        assert!(paths.validate(false).is_ok());
    }

    #[test]
    fn rejects_relative_tilde_traversal_and_outside_paths() {
        for invalid in ["relative", "~/state", "/var/lib/aletheon/../tmp"] {
            let paths = ProductionPaths {
                goals: PathBuf::from(invalid),
                ..ProductionPaths::default()
            };
            assert!(paths.validate(false).is_err(), "accepted {invalid}");
        }
        let paths = ProductionPaths {
            audit: PathBuf::from("/tmp/audit"),
            ..ProductionPaths::default()
        };
        assert_eq!(
            paths.validate(false),
            Err(PathContractError::OutsideApprovedRoot("audit"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn rejects_world_writable_data_and_symlinked_secret_roots() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let temp = tempfile::tempdir().unwrap();
        let world = temp.path().join("world");
        std::fs::create_dir(&world).unwrap();
        std::fs::set_permissions(&world, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert_eq!(
            validate_existing_directory(&world, "test", true, false),
            Err(PathContractError::WorldWritable("test"))
        );
        let target = temp.path().join("target");
        let linked = temp.path().join("linked");
        std::fs::create_dir(&target).unwrap();
        symlink(&target, &linked).unwrap();
        assert_eq!(
            validate_existing_directory(&linked, "secret", true, true),
            Err(PathContractError::Symlinked("secret"))
        );
    }

    #[test]
    fn missing_directories_are_mode_dependent() {
        let missing = PathBuf::from("/definitely-missing-aletheon-test-path");
        assert!(validate_existing_directory(&missing, "test", false, false).is_ok());
        assert_eq!(
            validate_existing_directory(&missing, "test", true, false),
            Err(PathContractError::Missing("test"))
        );
    }

    #[test]
    fn development_user_paths_remain_home_relative() {
        let home = home_dir();
        assert!(config_dir().starts_with(&home));
        assert!(xdg_config_dir().starts_with(&home));
        assert!(xdg_data_dir().starts_with(&home));
    }
}
