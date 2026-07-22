//! Explicit conversions between gRPC wire types and Fabric domain DTOs.
//!
//! Proto types never appear outside `hardware::grpc`. These free functions
//! are the only bridge between the wire contract and the domain model.

use crate::grpc::wire;
use fabric::types::embodiment as domain;

// ── RiskClass ────────────────────────────────────────────────────────────────

pub fn to_risk_class(v: i32) -> Result<domain::RiskClass, String> {
    match wire::RiskClass::try_from(v) {
        Ok(wire::RiskClass::Read) => Ok(domain::RiskClass::Read),
        Ok(wire::RiskClass::Low) => Ok(domain::RiskClass::Low),
        Ok(wire::RiskClass::Medium) => Ok(domain::RiskClass::Medium),
        Ok(wire::RiskClass::High) => Ok(domain::RiskClass::High),
        Ok(wire::RiskClass::Unspecified) | Err(_) => {
            Err(format!("unknown wire RiskClass value: {}", v))
        }
    }
}

// ── SkillOutcome ─────────────────────────────────────────────────────────────

pub fn to_skill_outcome(v: i32, failure_reason: String) -> Result<domain::SkillOutcome, String> {
    match wire::SkillOutcome::try_from(v) {
        Ok(wire::SkillOutcome::Succeeded) => Ok(domain::SkillOutcome::Succeeded),
        Ok(wire::SkillOutcome::Failed) => Ok(domain::SkillOutcome::Failed {
            reason: failure_reason,
        }),
        Ok(wire::SkillOutcome::Cancelled) => Ok(domain::SkillOutcome::Cancelled),
        Ok(wire::SkillOutcome::TimedOut) => Ok(domain::SkillOutcome::TimedOut),
        Ok(wire::SkillOutcome::Unspecified) | Err(_) => {
            Err(format!("unknown wire SkillOutcome value: {}", v))
        }
    }
}

// ── EvidenceRef ──────────────────────────────────────────────────────────────

pub fn to_evidence_ref(er: &wire::EvidenceRef) -> domain::EvidenceRef {
    domain::EvidenceRef {
        kind: er.kind.clone(),
        uri: er.uri.clone(),
    }
}

// ── SkillResult ──────────────────────────────────────────────────────────────

pub fn to_skill_result(sr: &wire::SkillResult) -> Result<domain::SkillResult, String> {
    let operation_id = sr
        .operation_id
        .parse()
        .map_err(|e| format!("invalid operation_id in SkillResult: {}", e))?;
    Ok(domain::SkillResult {
        operation_id,
        skill: domain::SkillId(sr.skill_id.clone()),
        device: domain::DeviceId(sr.device_id.clone()),
        outcome: to_skill_outcome(sr.outcome, sr.failure_reason.clone())?,
        duration_ms: sr.duration_ms,
        evidence: sr.evidence.iter().map(|e| to_evidence_ref(e)).collect(),
    })
}

// ── SkillProgress ────────────────────────────────────────────────────────────

pub fn to_skill_progress(sp: &wire::SkillProgress) -> Result<domain::SkillProgress, String> {
    let operation_id = sp
        .operation_id
        .parse()
        .map_err(|e| format!("invalid operation_id in SkillProgress: {}", e))?;
    Ok(domain::SkillProgress {
        operation_id,
        skill: domain::SkillId(sp.skill_id.clone()),
        fraction: sp.fraction,
        note: sp.note.clone(),
        at: fabric::MonoTime(sp.at_unix_ms.max(0) as u64),
    })
}

// ── SkillDescriptor ──────────────────────────────────────────────────────────

pub fn to_skill_descriptor(
    sd: &wire::SkillDescriptor,
) -> Result<domain::SkillDescriptor, String> {
    Ok(domain::SkillDescriptor {
        skill: domain::SkillId(sd.skill_id.clone()),
        device: domain::DeviceId(sd.device_id.clone()),
        summary: sd.summary.clone(),
        input_schema: struct_to_json(&sd.input_schema),
        risk: to_risk_class(sd.risk)?,
        timeout_ms: sd.timeout_ms,
        cancellable: sd.cancellable,
        preconditions: sd.preconditions.clone(),
        success_criteria: sd.success_criteria.clone(),
    })
}

// ── Observation ──────────────────────────────────────────────────────────────

pub fn to_observation(obs: &wire::Observation) -> Result<domain::EmbodiedObservation, String> {
    Ok(domain::EmbodiedObservation {
        schema: obs.schema.clone(),
        schema_version: obs.schema_version as u16,
        source: obs.source.clone(),
        sequence: obs.sequence,
        source_time: fabric::MonoTime(obs.source_unix_ms.max(0) as u64),
        received_at: fabric::MonoTime(obs.received_unix_ms.max(0) as u64),
        valid_until: if obs.valid_until_unix_ms > 0 {
            let delta = (obs.valid_until_unix_ms - obs.received_unix_ms).max(0) as u64;
            Some(fabric::MonoDeadline::after(
                fabric::MonoTime(obs.received_unix_ms.max(0) as u64),
                delta,
            ))
        } else {
            None
        },
        confidence: obs.confidence,
        frame_ref: if obs.frame_ref.is_empty() {
            None
        } else {
            Some(obs.frame_ref.clone())
        },
        payload: struct_to_json(&obs.payload),
        evidence: obs.evidence.iter().map(|e| to_evidence_ref(e)).collect(),
    })
}

// ── Struct → JSON ───────────────────────────────────────────────────────────

/// Convert serde_json::Value to a protobuf Struct.
pub fn json_to_struct(value: &serde_json::Value) -> prost_types::Struct {
    let mut fields = std::collections::BTreeMap::new();
    if let serde_json::Value::Object(map) = value {
        for (key, val) in map {
            fields.insert(key.clone(), json_to_prost_value(val));
        }
    }
    prost_types::Struct { fields }
}

fn json_to_prost_value(v: &serde_json::Value) -> prost_types::Value {
    use prost_types::value::Kind;
    let kind = match v {
        serde_json::Value::Null => Kind::NullValue(0),
        serde_json::Value::Bool(b) => Kind::BoolValue(*b),
        serde_json::Value::Number(n) => {
            Kind::NumberValue(n.as_f64().unwrap_or(0.0))
        }
        serde_json::Value::String(s) => Kind::StringValue(s.clone()),
        serde_json::Value::Array(arr) => {
            let values: Vec<prost_types::Value> = arr.iter().map(json_to_prost_value).collect();
            Kind::ListValue(prost_types::ListValue { values })
        }
        serde_json::Value::Object(_) => Kind::StructValue(json_to_struct(v)),
    };
    prost_types::Value { kind: Some(kind) }
}

/// Convert a protobuf Struct to serde_json::Value by traversing fields manually.
/// prost_types does not provide serde support by default.
fn struct_to_json(s: &Option<prost_types::Struct>) -> serde_json::Value {
    let s = match s {
        Some(s) => s,
        None => return serde_json::Value::Object(Default::default()),
    };
    let mut map = serde_json::Map::new();
    for (key, value) in &s.fields {
        map.insert(key.clone(), prost_value_to_json(value));
    }
    serde_json::Value::Object(map)
}

fn prost_value_to_json(v: &prost_types::Value) -> serde_json::Value {
    use prost_types::value::Kind;
    match &v.kind {
        Some(Kind::NullValue(_)) => serde_json::Value::Null,
        Some(Kind::NumberValue(n)) => {
            serde_json::Value::Number(serde_json::Number::from_f64(*n).unwrap_or_else(|| {
                serde_json::Number::from(0)
            }))
        }
        Some(Kind::StringValue(s)) => serde_json::Value::String(s.clone()),
        Some(Kind::BoolValue(b)) => serde_json::Value::Bool(*b),
        Some(Kind::StructValue(s)) => struct_to_json(&Some(s.clone())),
        Some(Kind::ListValue(list)) => {
            let values: Vec<serde_json::Value> =
                list.values.iter().map(prost_value_to_json).collect();
            serde_json::Value::Array(values)
        }
        None => serde_json::Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn risk_class_all_variants() {
        assert_eq!(
            to_risk_class(wire::RiskClass::Read as i32).unwrap(),
            domain::RiskClass::Read
        );
        assert_eq!(
            to_risk_class(wire::RiskClass::Low as i32).unwrap(),
            domain::RiskClass::Low
        );
        assert_eq!(
            to_risk_class(wire::RiskClass::Medium as i32).unwrap(),
            domain::RiskClass::Medium
        );
        assert_eq!(
            to_risk_class(wire::RiskClass::High as i32).unwrap(),
            domain::RiskClass::High
        );
    }

    #[test]
    fn unknown_risk_class_is_rejected() {
        assert!(to_risk_class(0).is_err());
        assert!(to_risk_class(99).is_err());
    }

    #[test]
    fn skill_outcome_all_variants() {
        assert_eq!(
            to_skill_outcome(wire::SkillOutcome::Succeeded as i32, String::new()).unwrap(),
            domain::SkillOutcome::Succeeded
        );
        let failed = to_skill_outcome(wire::SkillOutcome::Failed as i32, "motor fault".into()).unwrap();
        assert!(matches!(failed, domain::SkillOutcome::Failed { reason } if reason == "motor fault"));
        assert_eq!(
            to_skill_outcome(wire::SkillOutcome::Cancelled as i32, String::new()).unwrap(),
            domain::SkillOutcome::Cancelled
        );
        assert_eq!(
            to_skill_outcome(wire::SkillOutcome::TimedOut as i32, String::new()).unwrap(),
            domain::SkillOutcome::TimedOut
        );
    }

    #[test]
    fn unknown_skill_outcome_is_rejected() {
        assert!(to_skill_outcome(0, String::new()).is_err());
        assert!(to_skill_outcome(99, String::new()).is_err());
    }

    #[test]
    fn skill_result_preserves_identity() {
        let wr = wire::SkillResult {
            operation_id: "00000000-0000-4000-8000-000000000001".into(),
            skill_id: "kuavo.stance".into(),
            device_id: "kuavo-mujoco-01".into(),
            outcome: wire::SkillOutcome::Succeeded as i32,
            duration_ms: 150,
            ..Default::default()
        };
        let dr = to_skill_result(&wr).unwrap();
        assert_eq!(dr.skill.0, "kuavo.stance");
        assert_eq!(dr.device.0, "kuavo-mujoco-01");
        assert_eq!(dr.duration_ms, 150);
        assert_eq!(dr.outcome, domain::SkillOutcome::Succeeded);
    }

    #[test]
    fn progress_fraction_preserved() {
        let wp = wire::SkillProgress {
            operation_id: "00000000-0000-4000-8000-000000000002".into(),
            skill_id: "kuavo.move".into(),
            fraction: 0.75,
            note: "mid-way".into(),
            at_unix_ms: 5000,
        };
        let dp = to_skill_progress(&wp).unwrap();
        assert_eq!(dp.fraction, 0.75);
        assert_eq!(dp.note, "mid-way");
    }

    #[test]
    fn struct_to_json_round_trips_numbers_and_strings() {
        let mut fields = std::collections::BTreeMap::new();
        fields.insert(
            "x".into(),
            prost_types::Value {
                kind: Some(prost_types::value::Kind::NumberValue(1.5)),
            },
        );
        fields.insert(
            "mode".into(),
            prost_types::Value {
                kind: Some(prost_types::value::Kind::StringValue("stance".into())),
            },
        );
        let s = prost_types::Struct { fields };
        let json = struct_to_json(&Some(s));
        assert_eq!(json["x"].as_f64().unwrap(), 1.5);
        assert_eq!(json["mode"].as_str().unwrap(), "stance");
    }

    #[test]
    fn empty_struct_gives_empty_object() {
        let json = struct_to_json(&None);
        assert_eq!(json, serde_json::Value::Object(Default::default()));
    }
}
