//! Windows host capability backend (H2). Implements platform traits
//! with Win32 primitives: Job Objects, ConPTY, SCM, Named Pipes, ACL.

#[cfg(target_os = "windows")]
mod filesystem_host;
#[cfg(target_os = "windows")]
mod probe;
#[cfg(target_os = "windows")]
mod process_host;
#[cfg(target_os = "windows")]
mod pty_host;
#[cfg(target_os = "windows")]
mod service_host;

#[cfg(not(target_os = "windows"))]
mod stub;

#[cfg(target_os = "windows")]
pub use probe::WindowsBackend;
#[cfg(not(target_os = "windows"))]
pub use stub::WindowsStubBackend;
