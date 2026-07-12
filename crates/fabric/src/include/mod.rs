//! Subsystem trait contracts — like Linux kernel's include/ directory.
//!
//! Each file defines the trait contract for one subsystem.

pub mod admission;
pub mod agora;
pub mod body;
pub mod capability_invoker;
pub mod cognit;
pub mod event_bus;
pub mod memory;
pub mod meta;
pub mod plugin;
pub mod runtime;
pub mod self_field;
pub mod space;
pub mod subsystem;

pub mod turn;

pub mod chronos;

pub mod process;

pub mod compaction;
