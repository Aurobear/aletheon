//! Host-owned configuration for deterministic task evaluation settlement.

use fabric::{EvaluationContractError, EvaluationMode, EvaluationThresholds};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct EvaluationSettings {
    pub enabled: bool,
    pub default_mode: EvaluationMode,
    pub coding_rubric: String,
    pub min_score_millis: u32,
    pub min_evidence_coverage_millis: u16,
    pub min_confidence_millis: u16,
    pub max_evaluation_ms: u64,
}

impl Default for EvaluationSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            default_mode: EvaluationMode::Shadow,
            coding_rubric: "coding-v2".into(),
            min_score_millis: 70_000,
            min_evidence_coverage_millis: 600,
            min_confidence_millis: 700,
            max_evaluation_ms: 5_000,
        }
    }
}

impl EvaluationSettings {
    pub fn thresholds(&self) -> EvaluationThresholds {
        EvaluationThresholds {
            min_score_millis: self.min_score_millis,
            min_evidence_coverage_millis: self.min_evidence_coverage_millis,
            min_confidence_millis: self.min_confidence_millis,
        }
    }

    pub fn validate(&self) -> Result<(), EvaluationContractError> {
        self.thresholds().validate()?;
        if self.coding_rubric.trim().is_empty() {
            return Err(EvaluationContractError::EmptyField("coding_rubric"));
        }
        if self.max_evaluation_ms == 0 {
            return Err(EvaluationContractError::InvalidThreshold(
                "max_evaluation_ms",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evaluation_defaults_disabled_and_shadow() {
        let settings = EvaluationSettings::default();
        assert!(!settings.enabled);
        assert_eq!(settings.default_mode, EvaluationMode::Shadow);
        assert_eq!(settings.coding_rubric, "coding-v2");
        assert_eq!(settings.min_score_millis, 70_000);
        settings.validate().unwrap();
    }

    #[test]
    fn evaluation_rejects_invalid_thresholds_and_timeout() {
        let mut settings = EvaluationSettings {
            min_score_millis: 100_001,
            ..EvaluationSettings::default()
        };
        assert!(settings.validate().is_err());
        settings.min_score_millis = 70_000;
        settings.max_evaluation_ms = 0;
        assert!(settings.validate().is_err());
    }
}
