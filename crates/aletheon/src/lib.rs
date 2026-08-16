//! Pure host routing contracts used by the launcher and tests.

pub mod composition;
/// Binary-owned configuration surface: typed layered application
/// configuration, normalization, diagnostics and schema. Domain-owned
/// sub-configuration values are re-exported from their owner crates.
pub mod config;
/// Binary-owned diagnostic surface.
pub mod doctor {
    pub use crate::wiring::doctor::*;
}
/// Binary-owned extension package inspection surface.
///
/// Package inspection is deliberately local and read-only. Mutating
/// extension lifecycle operations remain daemon application use cases and are
/// reached through the typed Gateway command path.
pub mod extension {
    pub use crate::wiring::extension::inspect_archive;
}
/// Binary-owned extension lifecycle, install, snapshot and runtime routing.
pub mod extensions;
/// The binary-owned composition boundary for core, daemon, exec, and
/// user-daemon lifecycle. Production callers enter through this narrow facade.
pub mod launcher;
pub mod scoreboard;
/// Explicit binary wiring for the machine core and user daemon.  This module
/// is the first CGP-08 implementation slice; the remaining exec/extension
/// wiring follows the same direction.
pub mod wiring;
pub mod workspace;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandKind {
    Core,
    Daemon,
    Exec,
    Version,
    RestoreTerminal,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostRoute {
    Core,
    Daemon,
    Exec,
    Message,
    Tui,
    Version,
    RestoreTerminal,
}

pub const fn select_host(command: Option<CommandKind>, has_message: bool) -> HostRoute {
    match command {
        Some(CommandKind::Core) => HostRoute::Core,
        Some(CommandKind::Daemon) => HostRoute::Daemon,
        Some(CommandKind::Exec) => HostRoute::Exec,
        Some(CommandKind::Version) => HostRoute::Version,
        Some(CommandKind::RestoreTerminal) => HostRoute::RestoreTerminal,
        None if has_message => HostRoute::Message,
        None => HostRoute::Tui,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitStatus(pub u8);
impl ExitStatus {
    pub const fn from_success(success: bool) -> Self {
        Self(if success { 0 } else { 1 })
    }
}
