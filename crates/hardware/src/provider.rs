use crate::TypedCommand;

/// Marker created only after the Hardware domain validator accepts a command.
pub struct ValidatedCommand<'a>(pub(crate) &'a TypedCommand);
impl<'a> ValidatedCommand<'a> {
    pub fn command(&self) -> &TypedCommand {
        self.0
    }
}

/// Providers apply validated device intent only. They never issue permits or leases.
pub trait DeviceProvider {
    type Error;
    fn apply(&mut self, command: ValidatedCommand<'_>) -> Result<(), Self::Error>;
    fn safe_stop(&mut self) -> Result<(), Self::Error>;
}
