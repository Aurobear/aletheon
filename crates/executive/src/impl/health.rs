//! Sanitized production liveness/readiness model.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthClass {
    Ready,
    OptionalDegraded,
    RequiredUnready,
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentHealth {
    pub class: HealthClass,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub age_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_category: Option<&'static str>,
}

impl ComponentHealth {
    pub fn ready() -> Self {
        Self {
            class: HealthClass::Ready,
            count: None,
            age_seconds: None,
            error_category: None,
        }
    }

    pub fn disabled() -> Self {
        Self {
            class: HealthClass::Disabled,
            count: None,
            age_seconds: None,
            error_category: None,
        }
    }

    pub fn degraded(category: &'static str) -> Self {
        Self {
            class: HealthClass::OptionalDegraded,
            count: None,
            age_seconds: None,
            error_category: Some(category),
        }
    }

    pub fn unready(category: &'static str) -> Self {
        Self {
            class: HealthClass::RequiredUnready,
            count: None,
            age_seconds: None,
            error_category: Some(category),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductionHealth {
    pub liveness: &'static str,
    pub readiness: &'static str,
    pub components: BTreeMap<&'static str, ComponentHealth>,
}

#[derive(Default)]
pub struct HealthRegistry {
    components: Mutex<BTreeMap<&'static str, ComponentHealth>>,
    shutting_down: AtomicBool,
}

impl HealthRegistry {
    pub fn production_ready() -> Self {
        let registry = Self::default();
        for component in [
            "objective_store",
            "approvals_outbox",
            "local_memory",
            "worktree_capacity",
            "disk_quota",
        ] {
            registry.set(component, ComponentHealth::ready());
        }
        for component in [
            "telegram",
            "google_sync",
            "supplemental_memory_spool",
            "goal_worker",
            "backup",
        ] {
            registry.set(component, ComponentHealth::disabled());
        }
        registry
    }

    pub fn set(&self, component: &'static str, health: ComponentHealth) {
        self.components.lock().unwrap().insert(component, health);
    }

    pub fn begin_shutdown(&self) {
        self.shutting_down.store(true, Ordering::Release);
    }

    pub fn refresh_storage(
        &self,
        data_root: &Path,
        minimum_free_bytes: u64,
        backup_required: bool,
        maximum_backup_age_secs: u64,
    ) {
        match filesystem_free_bytes(data_root) {
            Ok(free) if free < minimum_free_bytes => {
                let mut health = ComponentHealth::unready("minimum_free_space");
                health.count = Some(free);
                self.set("disk_quota", health.clone());
                self.set("worktree_capacity", health);
            }
            Ok(free) => {
                let mut health = ComponentHealth::ready();
                health.count = Some(free);
                self.set("disk_quota", health.clone());
                self.set("worktree_capacity", health);
            }
            Err(()) => {
                self.set("disk_quota", ComponentHealth::unready("stat_failed"));
                self.set("worktree_capacity", ComponentHealth::unready("stat_failed"));
            }
        }

        if !backup_required {
            self.set("backup", ComponentHealth::disabled());
            return;
        }
        let marker = data_root.join("state/backup-marker.json");
        let age = marker
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .map(|elapsed| elapsed.as_secs());
        let mut health = match age {
            Some(age) if age <= maximum_backup_age_secs => ComponentHealth::ready(),
            Some(_) => ComponentHealth::degraded("backup_overdue"),
            None => ComponentHealth::degraded("backup_missing"),
        };
        health.age_seconds = age;
        self.set("backup", health);
    }

    pub fn snapshot(&self) -> ProductionHealth {
        let mut components = self.components.lock().unwrap().clone();
        if self.shutting_down.load(Ordering::Acquire) {
            components.insert("daemon", ComponentHealth::unready("shutting_down"));
        }
        let required_unready = components
            .values()
            .any(|component| component.class == HealthClass::RequiredUnready);
        let degraded = components
            .values()
            .any(|component| component.class == HealthClass::OptionalDegraded);
        ProductionHealth {
            liveness: "alive",
            readiness: if required_unready {
                "unready"
            } else if degraded {
                "degraded"
            } else {
                "ready"
            },
            components,
        }
    }
}

#[cfg(unix)]
fn filesystem_free_bytes(path: &Path) -> Result<u64, ()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let path = CString::new(path.as_os_str().as_bytes()).map_err(|_| ())?;
    let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: `path` is NUL-terminated and `stats` points to writable storage.
    if unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) } != 0 {
        return Err(());
    }
    // SAFETY: successful statvfs initialized the structure.
    let stats = unsafe { stats.assume_init() };
    Ok(stats.f_bavail.saturating_mul(stats.f_frsize))
}

#[cfg(not(unix))]
fn filesystem_free_bytes(_: &Path) -> Result<u64, ()> {
    Err(())
}
