// Integration tests for aletheon production validation

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

static INSTALLED_RUNTIME_LOCK: Mutex<()> = Mutex::new(());

pub(crate) fn installed_runtime_lock() -> MutexGuard<'static, ()> {
    INSTALLED_RUNTIME_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub(crate) fn official_user_socket() -> PathBuf {
    ::contracts::paths::UserRuntimePaths::resolve(&::contracts::paths::ProcessRuntimeEnvironment)
        .expect("official user runtime paths should resolve")
        .socket_path()
}

mod api_stress;
mod daemon_lifecycle;
mod socket_auth;
