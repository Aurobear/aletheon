//! Generic registry trait — like Linux kernel's `file_operations` registration.

use crate::AgentError;

/// Opaque handle returned when an item is registered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RegistrationId(pub u64);

/// A registry of named items.
///
/// Implementors store items by name and return a `RegistrationId` handle
/// for later unregister.
pub trait Registry<T> {
    /// Register an item. Returns a handle for unregistration.
    fn register(&mut self, item: T) -> Result<RegistrationId, AgentError>;

    /// Remove a previously registered item by its handle.
    fn unregister(&mut self, id: RegistrationId) -> Result<T, AgentError>;

    /// Look up an item by name.
    fn get(&self, name: &str) -> Option<&T>;

    /// Check whether an item with this name exists.
    fn contains(&self, name: &str) -> bool;

    /// List all registered names.
    fn names(&self) -> Vec<&str>;

    /// Number of registered items.
    fn len(&self) -> usize;

    /// Whether the registry is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
