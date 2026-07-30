//! Authoritative evidence records produced by runtime and tool adapters.

use std::collections::HashMap;

use fabric::OperationId;
use serde::{Deserialize, Serialize};

use super::cognitive_task::{AgentRuntimeId, EvidenceId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum EvidenceLevel {
    Located,
    Observed,
    Implemented,
    ProductionWired,
    DeterministicallyVerified,
    RuntimeVerified,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvidenceSource {
    Tool { name: String },
    AgentRuntime { runtime: AgentRuntimeId },
    Validation { name: String },
    HostRuntime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TerminalStatus {
    Pending,
    Succeeded,
    Failed,
    Cancelled,
}

impl TerminalStatus {
    pub fn is_terminal(self) -> bool {
        !matches!(self, Self::Pending)
    }

    pub fn is_success(self) -> bool {
        matches!(self, Self::Succeeded)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvidenceLocator {
    Operation(OperationId),
    FileRange {
        path: String,
        start_line: u32,
        end_line: u32,
    },
    Artifact {
        artifact_id: String,
    },
    DurableReceipt {
        receipt_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvidenceSubject {
    RoleGraphReceipt {
        root_task_id: String,
    },
    AgentInvocation {
        runtime: AgentRuntimeId,
    },
    ToolInvocation {
        tool_name: String,
    },
    OperationTerminal {
        operation_id: OperationId,
    },
    CommandSessionTerminal {
        session_id: String,
    },
    Deliverable {
        deliverable_id: String,
    },
    Validation {
        requirement_id: String,
    },
    ChangeDiffReview {
        transaction_id: String,
        workspace_version: String,
    },
    ChangeValidation {
        transaction_id: String,
        workspace_version: String,
    },
    ChangeAcceptance {
        transaction_id: String,
        workspace_version: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceRecord {
    pub id: EvidenceId,
    pub subject: EvidenceSubject,
    pub source: EvidenceSource,
    pub level: EvidenceLevel,
    pub terminal_status: TerminalStatus,
    pub locator: EvidenceLocator,
    pub digest: Option<String>,
}

impl EvidenceRecord {
    pub fn proves_success(&self) -> bool {
        self.terminal_status.is_success()
    }
}

#[derive(Debug, Default, Clone)]
pub struct EvidenceLedger {
    records: HashMap<EvidenceId, EvidenceRecord>,
}

impl EvidenceLedger {
    pub fn record(&mut self, evidence: EvidenceRecord) -> Option<EvidenceRecord> {
        self.records.insert(evidence.id.clone(), evidence)
    }

    pub fn get(&self, id: &EvidenceId) -> Option<&EvidenceRecord> {
        self.records.get(id)
    }

    pub fn records(&self) -> impl Iterator<Item = &EvidenceRecord> {
        self.records.values()
    }

    pub fn successful_for(&self, subject: &EvidenceSubject) -> Option<&EvidenceRecord> {
        self.records
            .values()
            .find(|record| &record.subject == subject && record.proves_success())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime_record(status: TerminalStatus) -> EvidenceRecord {
        let runtime = AgentRuntimeId("configured-runtime".into());
        EvidenceRecord {
            id: EvidenceId("receipt-1".into()),
            subject: EvidenceSubject::AgentInvocation {
                runtime: runtime.clone(),
            },
            source: EvidenceSource::AgentRuntime { runtime },
            level: EvidenceLevel::Observed,
            terminal_status: status,
            locator: EvidenceLocator::DurableReceipt {
                receipt_id: "receipt-1".into(),
            },
            digest: None,
        }
    }

    #[test]
    fn pending_submission_is_not_success_evidence() {
        let record = runtime_record(TerminalStatus::Pending);
        assert!(!record.terminal_status.is_terminal());
        assert!(!record.proves_success());
    }

    #[test]
    fn ledger_requires_matching_successful_terminal_record() {
        let mut ledger = EvidenceLedger::default();
        let pending = runtime_record(TerminalStatus::Pending);
        let subject = pending.subject.clone();
        ledger.record(pending);
        assert!(ledger.successful_for(&subject).is_none());

        ledger.record(runtime_record(TerminalStatus::Succeeded));
        assert!(ledger.successful_for(&subject).is_some());
    }
}
