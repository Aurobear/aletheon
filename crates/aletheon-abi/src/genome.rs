//! Genome types — like Linux kernel's device tree.
//!
//! The genome is the agent's self-description. Not code itself,
//! but the rules that generate code and runtime.

use serde::{Deserialize, Serialize};

/// Complete genome — the agent's self-description.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Genome {
    pub topology: Topology,
    pub identity: IdentitySpec,
    pub boundary: BoundarySpec,
    pub care: CareSpec,
    pub memory: MemorySpec,
    pub mutation: MutationSpec,
    pub lifecycle: LifecycleSpec,
}

/// Subsystem topology graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Topology {
    pub subsystems: Vec<SubsystemSpec>,
}

/// A subsystem in the topology.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubsystemSpec {
    pub name: String,
    pub subsystem_type: SubsystemType,
    pub version: String,
    pub dependencies: Vec<String>,
    pub config: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SubsystemType {
    Policy,       // SelfField
    Cognitive,    // BrainCore
    Execution,    // BodyRuntime
    Storage,      // Memory
    Infrastructure, // EventBus
    Evolution,    // MetaRuntime
}

/// Identity specification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentitySpec {
    pub name: String,
    pub description: String,
    pub self_model: String,
}

/// Boundary specification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoundarySpec {
    pub rules: Vec<BoundaryRuleSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoundaryRuleSpec {
    pub id: String,
    pub condition: String,
    pub action: String,
    pub priority: u32,
}

/// Care specification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CareSpec {
    pub priorities: Vec<CarePriority>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CarePriority {
    pub topic: String,
    pub weight: f64,
}

/// Memory specification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySpec {
    pub backends: Vec<String>,
    pub compaction_strategy: String,
}

/// Mutation specification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MutationSpec {
    pub allowed_targets: Vec<String>,
    pub require_sandbox: bool,
    pub require_self_field_approval: bool,
}

/// Lifecycle specification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LifecycleSpec {
    pub auto_compact: bool,
    pub health_check_interval_secs: u64,
    pub max_idle_time_secs: u64,
}
