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
        for component in ["telegram", "google_sync", "gbrain_spool", "backup"] {
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
