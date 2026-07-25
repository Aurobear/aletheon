//! Centralized development and production path contracts for Aletheon.

use std::ffi::OsString;
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

/// Environment lookup boundary used to resolve paths without mutating process
/// environment in tests or embedders.
pub trait RuntimeEnvironment {
    fn var_os(&self, key: &str) -> Option<OsString>;
}

/// Process environment used by production clients and per-user runtimes.
#[derive(Clone, Copy, Debug, Default)]
pub struct ProcessRuntimeEnvironment;

impl RuntimeEnvironment for ProcessRuntimeEnvironment {
    fn var_os(&self, key: &str) -> Option<OsString> {
        std::env::var_os(key)
    }
}

/// Private filesystem locations owned by one invoking user.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserRuntimePaths {
    pub runtime_root: PathBuf,
    pub state_root: PathBuf,
    pub cache_root: PathBuf,
}

impl UserRuntimePaths {
    /// Resolve user-private roots from injected XDG/HOME values.
    ///
    /// `XDG_RUNTIME_DIR` is mandatory because falling back to a shared runtime
    /// directory would make the control socket cross-user. State and cache use
    /// the standard HOME fallbacks when their XDG variables are absent.
    pub fn resolve(env: &impl RuntimeEnvironment) -> Result<Self, UserPathError> {
        let runtime =
            optional_env_path(env, "XDG_RUNTIME_DIR")?.ok_or(UserPathError::MissingRuntimeDir)?;
        let home = optional_env_path(env, "HOME")?;
        let state = match optional_env_path(env, "XDG_STATE_HOME")? {
            Some(path) => path,
            None => home
                .as_ref()
                .map(|path| path.join(".local/state"))
                .ok_or(UserPathError::MissingHome)?,
        };
        let cache = match optional_env_path(env, "XDG_CACHE_HOME")? {
            Some(path) => path,
            None => home
                .map(|path| path.join(".cache"))
                .ok_or(UserPathError::MissingHome)?,
        };
        Ok(Self {
            runtime_root: runtime.join("aletheon"),
            state_root: state.join("aletheon"),
            cache_root: cache.join("aletheon"),
        })
    }

    pub fn socket_path(&self) -> PathBuf {
        self.runtime_root.join("aletheon.sock")
    }

    /// Create private runtime/state/cache directories and verify that none is
    /// controlled by another OS user. Existing directories are never chmodded
    /// until their type and ownership have been verified.
    #[cfg(unix)]
    pub fn prepare(&self) -> Result<(), UserPathError> {
        prepare_user_directories(self, nix::unistd::geteuid().as_raw())
    }

    #[cfg(not(unix))]
    pub fn prepare(&self) -> Result<(), UserPathError> {
        Err(UserPathError::UnsupportedPlatform)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum UserPathError {
    #[error("XDG_RUNTIME_DIR is not set for the invoking user")]
    MissingRuntimeDir,
    #[error("HOME is not set and an XDG state or cache location is missing")]
    MissingHome,
    #[error("{0} must be an absolute path")]
    RelativePath(&'static str),
    #[error("{label} '{}' is a symbolic link", path.display())]
    Symlink { label: &'static str, path: PathBuf },
    #[error("{label} '{}' is not a directory", path.display())]
    NotDirectory { label: &'static str, path: PathBuf },
    #[error("{label} '{}' is owned by uid {actual}, expected uid {expected}", path.display())]
    WrongOwner {
        label: &'static str,
        path: PathBuf,
        expected: u32,
        actual: u32,
    },
    #[error("unable to {operation} {label} '{}': {source}", path.display())]
    Io {
        operation: &'static str,
        label: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("per-user runtime paths are only supported on Unix")]
    UnsupportedPlatform,
}

fn optional_env_path(
    env: &impl RuntimeEnvironment,
    key: &'static str,
) -> Result<Option<PathBuf>, UserPathError> {
    let Some(value) = env.var_os(key).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return Err(UserPathError::RelativePath(key));
    }
    Ok(Some(path))
}

#[cfg(unix)]
fn prepare_user_directories(
    paths: &UserRuntimePaths,
    effective_uid: u32,
) -> Result<(), UserPathError> {
    for (path, label) in [
        (&paths.runtime_root, "runtime root"),
        (&paths.state_root, "state root"),
        (&paths.cache_root, "cache root"),
    ] {
        prepare_user_directory(path, label, effective_uid)?;
    }
    Ok(())
}

#[cfg(unix)]
fn prepare_user_directory(
    path: &Path,
    label: &'static str,
    effective_uid: u32,
) -> Result<(), UserPathError> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

    match std::fs::symlink_metadata(path) {
        Ok(metadata) => verify_user_directory(path, label, &metadata, effective_uid)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut builder = std::fs::DirBuilder::new();
            builder.recursive(true).mode(0o700);
            builder.create(path).map_err(|source| UserPathError::Io {
                operation: "create",
                label,
                path: path.to_path_buf(),
                source,
            })?;
            let metadata = std::fs::symlink_metadata(path).map_err(|source| UserPathError::Io {
                operation: "inspect",
                label,
                path: path.to_path_buf(),
                source,
            })?;
            verify_user_directory(path, label, &metadata, effective_uid)?;
        }
        Err(source) => {
            return Err(UserPathError::Io {
                operation: "inspect",
                label,
                path: path.to_path_buf(),
                source,
            })
        }
    }

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).map_err(|source| {
        UserPathError::Io {
            operation: "set permissions on",
            label,
            path: path.to_path_buf(),
            source,
        }
    })
}

#[cfg(unix)]
fn verify_user_directory(
    path: &Path,
    label: &'static str,
    metadata: &std::fs::Metadata,
    effective_uid: u32,
) -> Result<(), UserPathError> {
    use std::os::unix::fs::MetadataExt;

    if metadata.file_type().is_symlink() {
        return Err(UserPathError::Symlink {
            label,
            path: path.to_path_buf(),
        });
    }
    if !metadata.is_dir() {
        return Err(UserPathError::NotDirectory {
            label,
            path: path.to_path_buf(),
        });
    }
    if metadata.uid() != effective_uid {
        return Err(UserPathError::WrongOwner {
            label,
            path: path.to_path_buf(),
            expected: effective_uid,
            actual: metadata.uid(),
        });
    }
    Ok(())
}

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
    std::env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join(".config"))
        .join("aletheon")
}

pub fn xdg_data_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join(".local/share"))
        .join("aletheon")
}

pub fn xdg_cache_dir() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join(".cache"))
        .join("aletheon")
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
    std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/nonexistent/aletheon-missing-home"))
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
    }

    #[test]
    fn rejects_relative_tilde_traversal_and_outside_paths() {
        for invalid in ["relative", "~/state", "/var/lib/aletheon/../tmp"] {
            assert!(
                validate_absolute(Path::new(invalid), "goals").is_err(),
                "accepted {invalid}"
            );
        }
        assert_eq!(
            validate_child(
                Path::new("/tmp/audit"),
                Path::new(PRODUCTION_STATE_ROOT),
                "audit"
            ),
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

    #[cfg(unix)]
    #[test]
    fn user_runtime_preparation_rejects_an_unexpected_owner() {
        let temp = tempfile::tempdir().unwrap();
        let paths = UserRuntimePaths {
            runtime_root: temp.path().join("runtime"),
            state_root: temp.path().join("state"),
            cache_root: temp.path().join("cache"),
        };
        let actual_uid = nix::unistd::geteuid().as_raw();
        let unexpected_uid = actual_uid.wrapping_add(1);
        let error = prepare_user_directories(&paths, unexpected_uid).unwrap_err();
        assert!(matches!(
            error,
            UserPathError::WrongOwner {
                expected,
                actual,
                ..
            } if expected == unexpected_uid && actual == actual_uid
        ));
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
