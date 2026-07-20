//! Public, bounded acceptance evidence for the conscious-core architecture.
//!
//! The trace intentionally contains causal identifiers and measurements only.
//! Hidden reasoning and free-form model self-report are not part of this schema.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const CONSCIOUS_CORE_TRACE_SCHEMA_V1: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndicatorResult {
    pub name: String,
    pub definition: String,
    pub baseline: f64,
    pub ablated: Option<f64>,
    pub passed: bool,
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConsciousTraceEvent {
    Candidate {
        disposition: String,
        content_id: String,
        source: String,
        salience: [f32; 8],
        policy_version: u32,
    },
    Broadcast {
        epoch: u64,
        winner_ids: Vec<String>,
        recipients: Vec<String>,
        acknowledgements: usize,
    },
    Integration {
        epoch: u64,
        dasein_before: u64,
        dasein_after: u64,
    },
    Prediction {
        prediction_id: String,
        surprised: bool,
        outcome_ref: String,
    },
    GovernedAction {
        operation_id: String,
        permit_ref: String,
        outcome_ref: String,
    },
    Memory {
        operation: String,
        receipt_ref: String,
        authority: String,
    },
    FieldModulation {
        mode: crate::ConsciousArbitrationMode,
        decision: crate::FieldDecisionKind,
        reason: crate::FieldDecisionReason,
        operation_id: String,
        call_id: String,
        broadcast_epoch: Option<u64>,
        baseline: Option<f64>,
        effective: Option<f64>,
        delta: Option<f64>,
        metric_ref: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConsciousCoreTrace {
    pub schema_version: u32,
    pub fixture_version: u32,
    pub events: Vec<ConsciousTraceEvent>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AcceptanceEvidence {
    pub fixture_version: u32,
    pub event_checksum: String,
    pub projection_checksums: BTreeMap<String, String>,
    pub indicator_results: Vec<IndicatorResult>,
    pub limitations: Vec<String>,
}
