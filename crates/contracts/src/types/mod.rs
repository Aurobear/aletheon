//! Shared data types used across all subsystems.

pub mod admission;
pub mod agent_control;
pub mod agent_settlement;
pub mod approval;
pub mod attempt;
pub mod capability;
pub mod change_transaction;
pub mod coding_job;
pub mod cognitive_workflow;
pub mod config_provenance;
pub mod conscious_arbitration;
pub mod conscious_core;
pub mod context;
pub mod context_budget;
pub mod data_governance;
pub mod embodiment;
pub mod episode_report;
pub mod evaluation;
pub mod evidence;
pub mod execution_target;
pub mod goal;
pub mod inference_receipt;
pub mod llm_types;
pub mod local_authority;
pub mod message;
pub mod metacognition_evaluation;
pub mod metacognition_evidence;
pub mod metacognition_experience;
pub mod model_projection;
pub mod paths;
pub mod permission;
pub mod sandbox;
pub mod session;
pub mod tool;
pub mod tool_stream;

pub mod operation;

pub mod turn;
pub mod turn_control;

pub mod process;
pub mod robot_failure;

pub mod space;

pub mod time;

pub mod workspace;

pub mod expected_outcome;
pub mod frame;
pub mod outcome_verification;
pub mod skill_proposal;
pub mod world_state;
