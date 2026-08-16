//! Binary-owned composition adapters for the Aletheon CLI and daemon.
//!
//! These seams construct Aletheon application services without making the
//! legacy `executive::composition` module a production caller.  The concrete
//! domain/application implementations remain in Aletheon until XRET-04;
//! ownership of composition is already local to this binary.

pub(crate) mod exec_corpus;
pub(crate) mod prefix_builder;
pub(crate) mod skill_admin;
pub(crate) mod turn_coordinator;
pub(crate) mod turn_service;

pub(crate) use crate::wiring::application::turn_engine::RuntimeIdentityBinder;
pub(crate) use turn_service::TurnService;

pub(crate) mod metacog_approval;

pub(crate) mod gmail_ingest_handler;

pub(crate) mod dasein_workspace;

pub(crate) mod evolution_proposer;

pub(crate) mod robot_harness;

pub(crate) mod harness_factory;
