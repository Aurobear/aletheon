//! Binary host staging for concrete aggregates awaiting port decomposition.

pub mod admin_service;
pub mod cognitive_runtime;
pub mod core;
pub mod doctor;
pub mod domain;
pub mod embodiment;
pub mod evolution_coordinator;
pub mod exec;
pub mod exec_session;
pub mod extension;
pub mod governed_review;
pub mod mode_router;
pub mod readiness;
pub mod request_use_cases;
pub mod runtime;
pub mod session;
pub mod turn_pipeline;
pub mod unix_server;
pub mod user_runtime;
pub mod workspace_checkpoint;
pub mod workspace_trust;
pub use admin_service::{AdminService, AdminUseCases};
pub mod goal_scheduler;
pub mod launcher;
