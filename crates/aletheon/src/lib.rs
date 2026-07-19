//! Pure host routing contracts used by the launcher and tests.

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
