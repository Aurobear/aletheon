use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use fabric::paths::{RuntimeEnvironment, UserPathError, UserRuntimePaths};

#[derive(Default)]
struct FakeEnv {
    values: BTreeMap<String, OsString>,
}

impl FakeEnv {
    fn new<const N: usize>(values: [(&str, &str); N]) -> Self {
        Self {
            values: values
                .into_iter()
                .map(|(key, value)| (key.to_string(), OsString::from(value)))
                .collect(),
        }
    }
}

impl RuntimeEnvironment for FakeEnv {
    fn var_os(&self, key: &str) -> Option<OsString> {
        self.values.get(key).cloned()
    }
}

#[test]
fn resolves_xdg_user_locations() {
    let env = FakeEnv::new([
        ("XDG_RUNTIME_DIR", "/run/user/1001"),
        ("XDG_STATE_HOME", "/home/a/.local/state"),
        ("XDG_CACHE_HOME", "/home/a/.cache"),
    ]);
    let paths = UserRuntimePaths::resolve(&env).unwrap();
    assert_eq!(
        paths.socket_path(),
        Path::new("/run/user/1001/aletheon/aletheon.sock")
    );
    assert_eq!(paths.state_root, Path::new("/home/a/.local/state/aletheon"));
    assert_eq!(paths.cache_root, Path::new("/home/a/.cache/aletheon"));
}

#[test]
fn falls_back_to_home_for_state_and_cache_only() {
    let env = FakeEnv::new([("XDG_RUNTIME_DIR", "/run/user/1001"), ("HOME", "/home/a")]);
    let paths = UserRuntimePaths::resolve(&env).unwrap();
    assert_eq!(paths.state_root, Path::new("/home/a/.local/state/aletheon"));
    assert_eq!(paths.cache_root, Path::new("/home/a/.cache/aletheon"));
}

#[test]
fn missing_xdg_runtime_dir_is_an_exact_error() {
    let error = UserRuntimePaths::resolve(&FakeEnv::default()).unwrap_err();
    assert_eq!(
        error.to_string(),
        "XDG_RUNTIME_DIR is not set for the invoking user"
    );
}

#[test]
fn runtime_dir_never_falls_back_to_home() {
    let env = FakeEnv::new([("HOME", "/home/a")]);
    assert!(matches!(
        UserRuntimePaths::resolve(&env),
        Err(UserPathError::MissingRuntimeDir)
    ));
}

#[cfg(unix)]
#[test]
fn prepare_creates_private_directories_without_global_environment_changes() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let temp = tempfile::tempdir().unwrap();
    let paths = UserRuntimePaths {
        runtime_root: temp.path().join("runtime/aletheon"),
        state_root: temp.path().join("state/aletheon"),
        cache_root: temp.path().join("cache/aletheon"),
    };
    paths.prepare().unwrap();

    for path in [&paths.runtime_root, &paths.state_root, &paths.cache_root] {
        let metadata = std::fs::symlink_metadata(path).unwrap();
        assert!(metadata.is_dir());
        assert_eq!(metadata.permissions().mode() & 0o777, 0o700);
        assert_eq!(metadata.uid(), nix::unistd::geteuid().as_raw());
    }
}

#[cfg(unix)]
#[test]
fn prepare_rejects_a_symlinked_runtime_root() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("target");
    std::fs::create_dir(&target).unwrap();
    let runtime_root = temp.path().join("runtime-root");
    symlink(&target, &runtime_root).unwrap();
    let paths = UserRuntimePaths {
        runtime_root: runtime_root.clone(),
        state_root: temp.path().join("state"),
        cache_root: temp.path().join("cache"),
    };

    let error = paths.prepare().unwrap_err();
    assert!(matches!(
        error,
        UserPathError::Symlink { path, .. } if path == runtime_root
    ));
}

#[test]
fn rejects_relative_injected_roots() {
    let env = FakeEnv::new([("XDG_RUNTIME_DIR", "run/user/1001"), ("HOME", "/home/a")]);
    assert!(matches!(
        UserRuntimePaths::resolve(&env),
        Err(UserPathError::RelativePath("XDG_RUNTIME_DIR"))
    ));
}

#[allow(dead_code)]
fn _path_type(_: PathBuf) {}
