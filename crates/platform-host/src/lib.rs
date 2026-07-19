//! Platform-host: backend selection and registry for host capability providers.
//! All backends implement platform-api traits; this crate selects the right one.

pub mod registry;
pub mod selector;

pub use registry::BackendRegistry;
pub use selector::{select_backend, HostPlatform};

use platform_api::HostCapabilityManifest;

/// Bootstrap: probe the running OS and select the correct backend.
pub fn probe() -> Result<HostCapabilityManifest, anyhow::Error> {
    let backend = select_backend();
    Ok(backend.probe())
}
