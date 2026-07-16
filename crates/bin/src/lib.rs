//! Pure host routing contracts used by the launcher and tests.

pub mod endpoint;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandKind {
    Daemon,
    Exec,
    Version,
    RestoreTerminal,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostRoute {
    Daemon,
    Exec,
    Message,
    Tui,
    Version,
    RestoreTerminal,
}

pub const fn select_host(command: Option<CommandKind>, has_message: bool) -> HostRoute {
    match command {
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
