//! Linux host capability backend (H1). Implements platform traits
//! with Linux-specific primitives: cgroup v2, pidfd, inotify, PTY, systemd.

#[cfg(target_os = "linux")]
mod filesystem_host;
#[cfg(target_os = "linux")]
mod probe;
#[cfg(target_os = "linux")]
mod process_host;
#[cfg(target_os = "linux")]
mod pty_host;
#[cfg(target_os = "linux")]
mod sandbox_host;
#[cfg(target_os = "linux")]
mod service_host;

#[cfg(not(target_os = "linux"))]
mod stub;

#[cfg(target_os = "linux")]
pub use filesystem_host::LinuxFilesystemHost;
#[cfg(target_os = "linux")]
pub use probe::LinuxBackend;
#[cfg(target_os = "linux")]
pub use process_host::LinuxProcessHost;
#[cfg(target_os = "linux")]
pub use pty_host::LinuxPtyHost;
#[cfg(target_os = "linux")]
pub use sandbox_host::LinuxSandboxHost;
#[cfg(target_os = "linux")]
pub use service_host::LinuxServiceHost;
#[cfg(not(target_os = "linux"))]
pub use stub::LinuxStubBackend;
