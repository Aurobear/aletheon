use serde::{Deserialize, Serialize};
use crate::manifest::RuntimeCapability;
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TaskKind { CodeModification, CodeReview, Research, ShellScript, DeviceOperation }

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AcceptanceCriterion {
    pub description: String,
    pub kind: AcceptanceKind,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum AcceptanceKind { TestsPass, NoRegression, DiffApproved, CommandSucceeded }

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VerificationPlan {
    pub criteria: Vec<AcceptanceCriterion>,
    pub required_evidence: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkOrder {
    pub objective: String,
    pub task_kind: TaskKind,
    pub acceptance_criteria: Vec<AcceptanceCriterion>,
    pub required_capabilities: BTreeSet<RuntimeCapability>,
    pub verification: VerificationPlan,
}
