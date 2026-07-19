//! macOS host capability backend (H3). Implements platform-api traits
//! with Darwin primitives: posix_spawn, FSEvents, launchd, Keychain, TCC.

#[cfg(target_os = "macos")]
mod probe;
#[cfg(target_os = "macos")]
mod process_host;
#[cfg(target_os = "macos")]
mod filesystem_host;
#[cfg(target_os = "macos")]
mod pty_host;
#[cfg(target_os = "macos")]
mod service_host;

#[cfg(not(target_os = "macos"))]
mod stub;

#[cfg(target_os = "macos")]
pub use probe::MacOSBackend;
#[cfg(not(target_os = "macos"))]
pub use stub::MacOSStubBackend;
