//! Stub backend for non-Windows hosts (compile-only target).

use crate::manifest::{FeatureState, HostCapabilityManifest, HostFeature};

pub struct WindowsStubBackend;

impl Default for WindowsStubBackend {
    fn default() -> Self {
        Self
    }
}

impl WindowsStubBackend {
    pub fn new() -> Self {
        Self
    }
    pub fn probe(&self) -> HostCapabilityManifest {
        HostCapabilityManifest {
            platform: "windows".into(),
            os_version: "stub (non-windows host)".into(),
            arch: std::env::consts::ARCH.into(),
            backend_version: env!("CARGO_PKG_VERSION").into(),
            features: vec![(HostFeature::ProcessTree, FeatureState::Unsupported)],
            probed_at_unix_ms: 0,
        }
    }
}
