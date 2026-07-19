//! Stub backend for non-macOS hosts.

use platform_api::manifest::{FeatureState, HostCapabilityManifest, HostFeature};

pub struct MacOSStubBackend;
impl MacOSStubBackend {
    pub fn new() -> Self { Self }
    pub fn probe(&self) -> HostCapabilityManifest {
        HostCapabilityManifest {
            platform: "macos".into(), os_version: "stub (non-macos host)".into(),
            arch: std::env::consts::ARCH.into(), backend_version: env!("CARGO_PKG_VERSION").into(),
            features: vec![(HostFeature::ProcessTree, FeatureState::Unsupported)],
            probed_at_unix_ms: 0,
        }
    }
}
