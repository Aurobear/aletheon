use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use aletheon_bin::endpoint::{resolve_client_socket, resolve_daemon_socket, resolve_socket};
use fabric::paths::{RuntimeEnvironment, UserRuntimePaths};

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

fn fixture_paths(runtime: &str) -> UserRuntimePaths {
    UserRuntimePaths {
        runtime_root: PathBuf::from(runtime).join("aletheon"),
        state_root: PathBuf::from("/home/a/.local/state/aletheon"),
        cache_root: PathBuf::from("/home/a/.cache/aletheon"),
    }
}

#[test]
fn endpoint_priority_is_cli_then_env_then_xdg() {
    let paths = fixture_paths("/run/user/1001");
    assert_eq!(
        resolve_socket(
            Some("/tmp/explicit.sock".into()),
            Some("/tmp/env.sock".into()),
            &paths,
        ),
        PathBuf::from("/tmp/explicit.sock")
    );
    assert_eq!(
        resolve_socket(None, Some("/tmp/env.sock".into()), &paths),
        PathBuf::from("/tmp/env.sock")
    );
    assert_eq!(resolve_socket(None, None, &paths), paths.socket_path());
}

#[test]
fn explicit_socket_does_not_require_xdg_runtime_dir() {
    let socket = resolve_client_socket(
        Some(PathBuf::from("/tmp/explicit.sock")),
        &FakeEnv::default(),
    )
    .unwrap();
    assert_eq!(socket, Path::new("/tmp/explicit.sock"));
}

#[test]
fn environment_socket_precedes_xdg_and_does_not_require_it() {
    let env = FakeEnv::new([("ALETHEON_SOCKET", "/tmp/environment.sock")]);
    assert_eq!(
        resolve_client_socket(None, &env).unwrap(),
        Path::new("/tmp/environment.sock")
    );
}

#[test]
fn default_socket_uses_private_xdg_runtime_path() {
    let env = FakeEnv::new([("XDG_RUNTIME_DIR", "/run/user/1001"), ("HOME", "/home/a")]);
    assert_eq!(
        resolve_client_socket(None, &env).unwrap(),
        Path::new("/run/user/1001/aletheon/aletheon.sock")
    );
}

#[test]
fn default_socket_without_xdg_has_the_exact_runtime_error() {
    let error = resolve_client_socket(None, &FakeEnv::default()).unwrap_err();
    assert_eq!(
        error.to_string(),
        "XDG_RUNTIME_DIR is not set for the invoking user"
    );
}

#[test]
fn daemon_subcommand_socket_overrides_parent_socket() {
    let socket = resolve_daemon_socket(
        Some(PathBuf::from("/tmp/daemon.sock")),
        Some(PathBuf::from("/tmp/parent.sock")),
        &FakeEnv::default(),
    )
    .unwrap();
    assert_eq!(socket, Path::new("/tmp/daemon.sock"));
}
