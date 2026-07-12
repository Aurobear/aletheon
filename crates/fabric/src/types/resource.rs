//! Managed resource — like Linux kernel's refcounted objects.
//!
//! Provides lifecycle-managed access to a value behind `Arc<Mutex<Option<T>>>`
//! with atomic state tracking.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use crate::error::AgentError;

/// Lifecycle state of a managed resource.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceState {
    Uninit = 0,
    Ready = 1,
    Shutting = 2,
    Dead = 3,
}

impl ResourceState {
    fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Uninit,
            1 => Self::Ready,
            2 => Self::Shutting,
            3 => Self::Dead,
            _ => Self::Dead,
        }
    }
}

/// A lifecycle-managed resource with atomic state tracking.
///
/// The resource starts `Uninit`, becomes `Ready` after `init()`,
/// transitions to `Shutting` during `shutdown()`, and finally `Dead`.
pub struct ManagedResource<T> {
    inner: Arc<Mutex<Option<T>>>,
    state: Arc<AtomicU8>,
    name: String,
}

impl<T> Clone for ManagedResource<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            state: Arc::clone(&self.state),
            name: self.name.clone(),
        }
    }
}

impl<T> ManagedResource<T> {
    /// Create a new resource in `Uninit` state.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(None)),
            state: Arc::new(AtomicU8::new(ResourceState::Uninit as u8)),
            name: name.into(),
        }
    }

    /// Create a new resource with an initial value (starts in `Ready` state).
    pub fn with_value(name: impl Into<String>, value: T) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Some(value))),
            state: Arc::new(AtomicU8::new(ResourceState::Ready as u8)),
            name: name.into(),
        }
    }

    /// Current lifecycle state.
    pub fn state(&self) -> ResourceState {
        ResourceState::from_u8(self.state.load(Ordering::Acquire))
    }

    /// Resource name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Initialize the resource with a value. Fails if not `Uninit`.
    pub fn init(&self, value: T) -> Result<(), AgentError> {
        let prev = self
            .state
            .compare_exchange(
                ResourceState::Uninit as u8,
                ResourceState::Ready as u8,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .map_err(|_| AgentError::already_exists(&format!("resource '{}'", self.name)))?;
        debug_assert_eq!(prev, ResourceState::Uninit as u8);
        let mut guard = self.inner.lock().unwrap();
        *guard = Some(value);
        Ok(())
    }

    /// Get a reference to the value. Returns `None` if not `Ready`.
    pub fn get(&self) -> Result<std::sync::MutexGuard<'_, Option<T>>, AgentError> {
        if self.state() != ResourceState::Ready {
            return Err(AgentError::not_found(&format!(
                "resource '{}' not ready (state: {:?})",
                self.name,
                self.state()
            )));
        }
        Ok(self.inner.lock().unwrap())
    }

    /// Transition to `Shutting` then `Dead`. Returns the inner value if it was `Ready`.
    pub fn shutdown(&self) -> Result<(), AgentError> {
        let prev = self
            .state
            .compare_exchange(
                ResourceState::Ready as u8,
                ResourceState::Shutting as u8,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .map_err(|_| {
                AgentError::not_found(&format!(
                    "resource '{}' not ready (state: {:?})",
                    self.name,
                    self.state()
                ))
            })?;
        debug_assert_eq!(prev, ResourceState::Ready as u8);
        // Drop the inner value.
        let mut guard = self.inner.lock().unwrap();
        *guard = None;
        self.state
            .store(ResourceState::Dead as u8, Ordering::Release);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_states() {
        let res = ManagedResource::<i32>::new("test");
        assert_eq!(res.state(), ResourceState::Uninit);
        assert_eq!(res.name(), "test");

        res.init(42).unwrap();
        assert_eq!(res.state(), ResourceState::Ready);

        {
            let guard = res.get().unwrap();
            assert_eq!(*guard, Some(42));
        }

        res.shutdown().unwrap();
        assert_eq!(res.state(), ResourceState::Dead);
    }

    #[test]
    fn get_before_init_fails() {
        let res = ManagedResource::<i32>::new("test");
        assert!(res.get().is_err());
    }

    #[test]
    fn double_init_fails() {
        let res = ManagedResource::<i32>::new("test");
        res.init(1).unwrap();
        assert!(res.init(2).is_err());
    }

    #[test]
    fn get_after_shutdown_fails() {
        let res = ManagedResource::<i32>::new("test");
        res.init(42).unwrap();
        res.shutdown().unwrap();
        assert!(res.get().is_err());
    }
}
