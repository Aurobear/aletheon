//! Chronos clock contracts.

use crate::types::time::{MonoTime, WallTime};

pub trait Clock: Send + Sync {
    fn wall_now(&self) -> WallTime;
    fn mono_now(&self) -> MonoTime;
}
