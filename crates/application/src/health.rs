//! Sanitized production liveness/readiness model.

use std::collections::BTreeMap;
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_digest: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub asset_counts: BTreeMap<String, u64>,
}

impl ComponentHealth {
    pub fn ready() -> Self {
        Self {
            class: HealthClass::Ready,
            count: None,
            age_seconds: None,
            error_category: None,
            items: Vec::new(),
            snapshot_digest: None,
            asset_counts: BTreeMap::new(),
        }
    }

    pub fn disabled() -> Self {
        Self {
            class: HealthClass::Disabled,
            count: None,
            age_seconds: None,
            error_category: None,
            items: Vec::new(),
            snapshot_digest: None,
            asset_counts: BTreeMap::new(),
        }
    }

    pub fn degraded(category: &'static str) -> Self {
        Self {
            class: HealthClass::OptionalDegraded,
            count: None,
            age_seconds: None,
            error_category: Some(category),
            items: Vec::new(),
            snapshot_digest: None,
            asset_counts: BTreeMap::new(),
        }
    }

    pub fn unready(category: &'static str) -> Self {
        Self {
            class: HealthClass::RequiredUnready,
            count: None,
            age_seconds: None,
            error_category: Some(category),
            items: Vec::new(),
            snapshot_digest: None,
            asset_counts: BTreeMap::new(),
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
            "external_sync",
            "supplemental_memory_spool",
            "goal_worker",
            "backup",
        ] {
            registry.set(component, ComponentHealth::disabled());
        }
        registry
    }

    pub fn set(&self, component: &'static str, health: ComponentHealth) {
        self.components
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(component, health);
    }

    pub fn begin_shutdown(&self) {
        self.shutting_down.store(true, Ordering::Release);
    }

    pub fn update_storage(
        &self,
        free_bytes: Option<u64>,
        minimum_free_bytes: u64,
        backup_required: bool,
        backup_age_seconds: Option<u64>,
        maximum_backup_age_secs: u64,
    ) {
        match free_bytes {
            Some(free) if free < minimum_free_bytes => {
                let mut health = ComponentHealth::unready("minimum_free_space");
                health.count = Some(free);
                self.set("disk_quota", health.clone());
                self.set("worktree_capacity", health);
            }
            Some(free) => {
                let mut health = ComponentHealth::ready();
                health.count = Some(free);
                self.set("disk_quota", health.clone());
                self.set("worktree_capacity", health);
            }
            None => {
                self.set("disk_quota", ComponentHealth::unready("stat_failed"));
                self.set("worktree_capacity", ComponentHealth::unready("stat_failed"));
            }
        }
        if !backup_required {
            self.set("backup", ComponentHealth::disabled());
            return;
        }
        let mut health = match backup_age_seconds {
            Some(age) if age <= maximum_backup_age_secs => ComponentHealth::ready(),
            Some(_) => ComponentHealth::degraded("backup_overdue"),
            None => ComponentHealth::degraded("backup_missing"),
        };
        health.age_seconds = backup_age_seconds;
        self.set("backup", health);
    }

    pub fn snapshot(&self) -> ProductionHealth {
        let mut components = self
            .components
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
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
