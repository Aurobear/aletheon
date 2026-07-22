//! Shared data types used across all subsystems.

pub mod admission;
pub mod agent;
pub mod agent_control;
pub mod agent_profile_event;
pub mod agent_settlement;
pub mod approval;
pub mod attempt;
pub mod capability;
pub mod channel;
pub mod coding_job;
pub mod conscious_arbitration;
pub mod conscious_core;
pub mod conscious_core_trace;
pub mod conscious_field_metrics;
pub mod context;
pub mod embodiment;
pub mod evidence;
pub mod extension;
pub mod external_event;
pub mod external_identity;
pub mod genome;
pub mod goal;
pub mod google;
pub mod grounding;
pub mod hook;
pub mod hook_ext;
pub mod lifecycle;
pub mod llm_types;
pub mod local_authority;
pub mod message;
pub mod network_policy;
pub mod objective;
pub mod paths;
pub mod permission;
pub mod prompt_queue;
pub mod resource;
pub mod sandbox;
pub mod sandbox_glob;
pub mod session;
pub mod tool;
pub mod tool_stream;
pub mod vision;

pub mod operation;

pub mod turn;

pub mod process;

pub mod space;

pub mod time;

pub mod workspace;

pub mod workspace_checkpoint;

pub mod workspace_trust;

pub mod expected_outcome;
pub mod outcome_verification;
pub mod world_state;
pub mod frame;
pub mod perception_observation;
pub mod skill_proposal;
pub mod embodied_episode;
pub mod hil_evidence;
pub mod emergency_stop;
pub mod robot_audit;
