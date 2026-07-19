//! Stub backend for non-Linux hosts (compile-only target).

use crate::manifest::{FeatureState, HostCapabilityManifest, HostFeature};

pub struct LinuxStubBackend;

impl Default for LinuxStubBackend {
    fn default() -> Self {
        Self
    }
}

impl LinuxStubBackend {
    pub fn new() -> Self {
        Self
    }

    pub fn probe(&self) -> HostCapabilityManifest {
        HostCapabilityManifest {
            platform: "linux".into(),
            os_version: "stub (non-linux host)".into(),
            arch: std::env::consts::ARCH.into(),
            backend_version: env!("CARGO_PKG_VERSION").into(),
            features: vec![(HostFeature::ProcessTree, FeatureState::Unsupported)],
            probed_at_unix_ms: 0,
        }
    }
}
