//! Linux host capability backend (H1). Implements platform-api traits
//! with Linux-specific primitives: cgroup v2, pidfd, inotify, PTY, systemd.

#[cfg(target_os = "linux")]
mod probe;
#[cfg(not(target_os = "linux"))]
mod stub;

#[cfg(target_os = "linux")]
pub use probe::LinuxBackend;
#[cfg(not(target_os = "linux"))]
pub use stub::LinuxStubBackend;
