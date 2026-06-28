//! EvolutionValidator — validates and applies behavior adjustments safely.
//!
//! Ensures that proposed adjustments from the evolution engine don't violate
//! core invariants (identity immutability, safety floors, weight ranges).
//! Provides snapshot/compare/rollback for safe experimentation.

use aletheon_abi::brain::BehaviorAdjustment;
use serde::{Deserialize, Serialize};

use crate::core::attention::FocusTopic;
use crate::core::SelfField;

/// Identity core values that cannot be adjusted by evolution.
const IDENTITY_CORE_TARGETS: &[&str] = &[
    "identity.name",
    "identity.description",
    "identity.version",
    "identity.created_at",
];

/// Minimum confidence required for a source reflection to trigger adjustments.
const MIN_CONFIDENCE: f64 = 0.5;

/// Safety floor: the "safety" care weight cannot go below this value.
const SAFETY_FLOOR: f64 = 0.8;

/// Valid range for care weights.
const CARE_WEIGHT_MIN: f64 = 0.1;
const CARE_WEIGHT_MAX: f64 = 1.0;

/// Result of validating a single behavior adjustment.
#[derive(Debug, Clone)]
pub enum ValidationResult {
    /// The adjustment passed all checks and can be applied as-is.
    Approved,
    /// The adjustment was rejected; reason explains why.
    Rejected { reason: String },
    /// The adjustment was modified to comply with safety constraints.
    Modified { adjusted: BehaviorAdjustment },
}

/// Snapshot of SelfField state for baseline comparison.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaselineSnapshot {
    /// Care topic -> weight at snapshot time.
    pub care_weights: Vec<(String, f64)>,
    /// Number of boundary rules at snapshot time.
    pub boundary_rule_count: usize,
    /// Active attention focus topics at snapshot time.
    pub attention_focus: Vec<FocusTopic>,
}

/// Outcome of comparing current state against a baseline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvolutionOutcome {
    /// The system has improved since the baseline.
    Improved,
    /// No meaningful change detected.
    NoChange,
    /// The system has degraded — triggers rollback.
    Degraded,
}

/// Validates, applies, and tracks behavior adjustments safely.
pub struct EvolutionValidator {
    /// Confidence of the current reflection driving adjustments.
    /// Set externally before calling validate_adjustment.
    pub reflection_confidence: f64,
}

impl EvolutionValidator {
    /// Create a new validator with the given source reflection confidence.
    pub fn new(reflection_confidence: f64) -> Self {
        Self {
            reflection_confidence,
        }
    }

    /// Create a validator with default (zero) confidence.
    pub fn default_confidence() -> Self {
        Self::new(0.0)
    }

    /// Validate a single behavior adjustment against the current SelfField state.
    ///
    /// Checks (in order):
    /// 1. Non-empty reason
    /// 2. Source reflection confidence > 0.5
    /// 3. Cannot adjust identity core values
    /// 4. Cannot relax boundary rules below safety floor (immutable rules)
    /// 5. Care weights must stay in [0.1, 1.0]; safety care >= 0.8
    pub fn validate_adjustment(
        &self,
        adjustment: &BehaviorAdjustment,
        self_field: &SelfField,
    ) -> ValidationResult {
        // Rule 1: reason must be non-empty
        if adjustment.reason.trim().is_empty() {
            return ValidationResult::Rejected {
                reason: "Adjustment has empty reason".to_string(),
            };
        }

        // Rule 2: confidence threshold
        if self.reflection_confidence <= MIN_CONFIDENCE {
            return ValidationResult::Rejected {
                reason: format!(
                    "Source reflection confidence {} is below threshold {}",
                    self.reflection_confidence, MIN_CONFIDENCE
                ),
            };
        }

        // Rule 3: cannot adjust identity core values
        if IDENTITY_CORE_TARGETS.iter().any(|&core| adjustment.target.starts_with(core)) {
            return ValidationResult::Rejected {
                reason: format!(
                    "Cannot adjust identity core value '{}'",
                    adjustment.target
                ),
            };
        }

        // Rule 4: boundary rule immutability
        if adjustment.target.starts_with("boundary.rule") {
            return self.validate_boundary_adjustment(adjustment, self_field);
        }

        // Rule 5: care weight validation
        if adjustment.target.starts_with("care.") {
            return self.validate_care_adjustment(adjustment, self_field);
        }

        // Unknown target — approve with a note (no specific rule violated)
        ValidationResult::Approved
    }

    /// Validate a boundary-related adjustment.
    fn validate_boundary_adjustment(
        &self,
        adjustment: &BehaviorAdjustment,
        self_field: &SelfField,
    ) -> ValidationResult {
        let boundary = self_field.boundary();

        // If new_value < old_value for a boundary rule, it's a relaxation
        if let (Some(old), Some(new)) = (adjustment.old_value, adjustment.new_value) {
            if new < old {
                // Check immutability via rule count (immutable rules can't be relaxed)
                // The boundary layer enforces immutability in relax_rule(), so we
                // just need to flag attempts that bypass it.
                if boundary.immutable_rule_count() > 0 {
                    return ValidationResult::Rejected {
                        reason: format!(
                            "Cannot relax boundary rule '{}' — immutable rules present",
                            adjustment.target
                        ),
                    };
                }
            }
        }

        ValidationResult::Approved
    }

    /// Validate a care-related adjustment.
    fn validate_care_adjustment(
        &self,
        adjustment: &BehaviorAdjustment,
        _self_field: &SelfField,
    ) -> ValidationResult {
        // Extract the care topic from the target (e.g., "care.safety.weight" -> "safety")
        let parts: Vec<&str> = adjustment.target.split('.').collect();
        if parts.len() < 2 {
            return ValidationResult::Rejected {
                reason: format!(
                    "Invalid care target format '{}', expected 'care.<topic>[.weight]'",
                    adjustment.target
                ),
            };
        }
        let topic = parts[1];

        if let Some(new_val) = adjustment.new_value {
            // Clamp to valid range
            let clamped = new_val.clamp(CARE_WEIGHT_MIN, CARE_WEIGHT_MAX);

            // Safety floor check
            let effective = if topic == "safety" {
                clamped.max(SAFETY_FLOOR)
            } else {
                clamped
            };

            if (effective - new_val).abs() > f64::EPSILON {
                let mut adjusted = adjustment.clone();
                adjusted.new_value = Some(effective);
                adjusted.reason = format!(
                    "{} (clamped from {} to {})",
                    adjustment.reason, new_val, effective
                );
                return ValidationResult::Modified { adjusted };
            }
        }

        ValidationResult::Approved
    }

    /// Apply a batch of adjustments to SelfField, validating each one.
    ///
    /// Only Approved and Modified adjustments are applied.
    pub fn apply_adjustments(
        &self,
        adjustments: &[BehaviorAdjustment],
        self_field: &mut SelfField,
    ) -> Vec<ValidationResult> {
        let mut results = Vec::with_capacity(adjustments.len());

        for adj in adjustments {
            let result = self.validate_adjustment(adj, self_field);

            match &result {
                ValidationResult::Approved => {
                    self.apply_single(adj, self_field);
                }
                ValidationResult::Modified { adjusted } => {
                    self.apply_single(adjusted, self_field);
                }
                ValidationResult::Rejected { .. } => {
                    // Skip rejected adjustments
                }
            }

            results.push(result);
        }

        results
    }

    /// Apply a single (already validated) adjustment to SelfField.
    fn apply_single(&self, adjustment: &BehaviorAdjustment, self_field: &mut SelfField) {
        if adjustment.target.starts_with("care.") {
            let parts: Vec<&str> = adjustment.target.split('.').collect();
            if parts.len() >= 2 {
                let topic = parts[1];
                if let (Some(old), Some(new)) = (adjustment.old_value, adjustment.new_value) {
                    let delta = new - old;
                    self_field.care_mut().adjust_weight(topic, delta);
                }
            }
        } else if adjustment.target.starts_with("boundary.") {
            if let Some(new_val) = adjustment.new_value {
                let parts: Vec<&str> = adjustment.target.split('.').collect();
                if parts.len() >= 3 {
                    let pattern = parts[2];
                    if new_val > 0.0 {
                        self_field.boundary_mut().tighten_rule(pattern);
                    } else {
                        self_field.boundary_mut().relax_rule(pattern);
                    }
                }
            }
        }
    }

    /// Capture the current SelfField state as a baseline snapshot.
    pub fn record_baseline(&self, self_field: &SelfField) -> BaselineSnapshot {
        let care_weights: Vec<(String, f64)> = self_field
            .care()
            .all_cares()
            .into_iter()
            .map(|c| (c.topic, c.weight))
            .collect();

        let boundary_rule_count = self_field.boundary().rule_count();

        let attention_focus = self_field.attention().all_topics();

        BaselineSnapshot {
            care_weights,
            boundary_rule_count,
            attention_focus,
        }
    }

    /// Compare current SelfField state against a baseline snapshot.
    ///
    /// Evaluation criteria:
    /// - Improved: more boundary rules OR higher average care weights
    /// - Degraded: fewer boundary rules OR lower average care weights
    /// - NoChange: negligible difference
    pub fn compare_with_baseline(
        &self,
        baseline: &BaselineSnapshot,
        self_field: &SelfField,
    ) -> EvolutionOutcome {
        let current = self.record_baseline(self_field);

        // Compare boundary rule counts
        let rule_delta = current.boundary_rule_count as i64 - baseline.boundary_rule_count as i64;

        // Compare average care weights
        let baseline_avg = average_weight(&baseline.care_weights);
        let current_avg = average_weight(&current.care_weights);
        let weight_delta = current_avg - baseline_avg;

        // Threshold for "meaningful change"
        const DELTA_THRESHOLD: f64 = 0.01;

        if rule_delta > 0 || weight_delta > DELTA_THRESHOLD {
            EvolutionOutcome::Improved
        } else if rule_delta < 0 || weight_delta < -DELTA_THRESHOLD {
            EvolutionOutcome::Degraded
        } else {
            EvolutionOutcome::NoChange
        }
    }

    /// Rollback SelfField state to a baseline snapshot.
    ///
    /// Restores care weights and boundary rule count. Attention focus
    /// is not rolled back as it represents real-time state.
    pub fn rollback(&self, baseline: &BaselineSnapshot, self_field: &mut SelfField) {
        // Restore care weights
        for (topic, target_weight) in &baseline.care_weights {
            if let Some(current_weight) = self_field.care().weight_of(topic) {
                let delta = target_weight - current_weight;
                if delta.abs() > f64::EPSILON {
                    self_field.care_mut().adjust_weight(topic, delta);
                }
            }
        }

        // Note: boundary rule rollback is intentionally not done here because
        // removing rules is destructive and could reduce safety. The snapshot
        // records the count for comparison, but rollback focuses on care weights
        // which are the primary tunable parameters.
    }
}

/// Compute average weight from a list of (topic, weight) pairs.
fn average_weight(weights: &[(String, f64)]) -> f64 {
    if weights.is_empty() {
        return 0.0;
    }
    let sum: f64 = weights.iter().map(|(_, w)| w).sum();
    sum / weights.len() as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{SelfField, SelfFieldConfig};
    use crate::core::boundary::{BoundaryAction, BoundaryRule};
    use aletheon_abi::self_field::RiskLevel;

    fn make_self_field() -> SelfField {
        SelfField::new(SelfFieldConfig::default())
    }

    fn make_adjustment(target: &str, old: Option<f64>, new: Option<f64>, reason: &str) -> BehaviorAdjustment {
        BehaviorAdjustment {
            target: target.to_string(),
            old_value: old,
            new_value: new,
            reason: reason.to_string(),
        }
    }

    // --- Validation rule tests ---

    #[test]
    fn reject_empty_reason() {
        let validator = EvolutionValidator::new(0.8);
        let sf = make_self_field();
        let adj = make_adjustment("care.efficiency.weight", Some(0.5), Some(0.7), "");
        let result = validator.validate_adjustment(&adj, &sf);
        assert!(matches!(result, ValidationResult::Rejected { .. }));
    }

    #[test]
    fn reject_low_confidence() {
        let validator = EvolutionValidator::new(0.3);
        let sf = make_self_field();
        let adj = make_adjustment("care.efficiency.weight", Some(0.5), Some(0.7), "boost efficiency");
        let result = validator.validate_adjustment(&adj, &sf);
        assert!(matches!(result, ValidationResult::Rejected { .. }));
    }

    #[test]
    fn reject_identity_core() {
        let validator = EvolutionValidator::new(0.8);
        let sf = make_self_field();
        let adj = make_adjustment("identity.name", None, None, "rename");
        let result = validator.validate_adjustment(&adj, &sf);
        assert!(matches!(result, ValidationResult::Rejected { .. }));
    }

    #[test]
    fn reject_identity_version() {
        let validator = EvolutionValidator::new(0.8);
        let sf = make_self_field();
        let adj = make_adjustment("identity.version", None, None, "bump version");
        let result = validator.validate_adjustment(&adj, &sf);
        assert!(matches!(result, ValidationResult::Rejected { .. }));
    }

    #[test]
    fn reject_boundary_relax_immutable() {
        let validator = EvolutionValidator::new(0.8);
        let mut sf = make_self_field();
        sf.boundary_mut().add_rule(BoundaryRule {
            action_pattern: "rm *".to_string(),
            source_filter: None,
            action: BoundaryAction::Deny,
            risk_level: RiskLevel::Critical,
            description: "no rm".to_string(),
            immutable: true,
        });

        let adj = make_adjustment("boundary.rule.rm *", Some(1.0), Some(0.0), "relax rm");
        let result = validator.validate_adjustment(&adj, &sf);
        assert!(matches!(result, ValidationResult::Rejected { .. }));
    }

    #[test]
    fn modify_care_weight_out_of_range() {
        let validator = EvolutionValidator::new(0.8);
        let sf = make_self_field();
        // Trying to set efficiency to 1.5, which is above max
        let adj = make_adjustment("care.efficiency.weight", Some(0.5), Some(1.5), "boost too high");
        let result = validator.validate_adjustment(&adj, &sf);
        assert!(matches!(result, ValidationResult::Modified { adjusted } if adjusted.new_value == Some(1.0)));
    }

    #[test]
    fn modify_safety_below_floor() {
        let validator = EvolutionValidator::new(0.8);
        let sf = make_self_field();
        // Trying to set safety below 0.8 floor
        let adj = make_adjustment("care.safety.weight", Some(1.0), Some(0.5), "reduce safety");
        let result = validator.validate_adjustment(&adj, &sf);
        assert!(matches!(result, ValidationResult::Modified { adjusted } if adjusted.new_value == Some(0.8)));
    }

    #[test]
    fn approve_valid_care_adjustment() {
        let validator = EvolutionValidator::new(0.8);
        let sf = make_self_field();
        let adj = make_adjustment("care.efficiency.weight", Some(0.5), Some(0.7), "boost efficiency");
        let result = validator.validate_adjustment(&adj, &sf);
        assert!(matches!(result, ValidationResult::Approved));
    }

    #[test]
    fn approve_unknown_target() {
        let validator = EvolutionValidator::new(0.8);
        let sf = make_self_field();
        let adj = make_adjustment("custom.unknown", None, Some(42.0), "custom tweak");
        let result = validator.validate_adjustment(&adj, &sf);
        assert!(matches!(result, ValidationResult::Approved));
    }

    // --- Apply tests ---

    #[test]
    fn apply_adjustments_applies_approved() {
        let validator = EvolutionValidator::new(0.8);
        let mut sf = make_self_field();

        let adj = make_adjustment("care.efficiency.weight", Some(0.5), Some(0.7), "boost efficiency");
        let results = validator.apply_adjustments(&[adj], &mut sf);

        assert_eq!(results.len(), 1);
        assert!(matches!(results[0], ValidationResult::Approved));
        // Efficiency should have moved by +0.2
        let weight = sf.care().weight_of("efficiency").unwrap();
        assert!((weight - 0.7).abs() < f64::EPSILON);
    }

    #[test]
    fn apply_adjustments_skips_rejected() {
        let validator = EvolutionValidator::new(0.3); // low confidence
        let mut sf = make_self_field();

        let original_weight = sf.care().weight_of("efficiency").unwrap();
        let adj = make_adjustment("care.efficiency.weight", Some(0.5), Some(0.7), "boost");
        let results = validator.apply_adjustments(&[adj], &mut sf);

        assert!(matches!(results[0], ValidationResult::Rejected { .. }));
        assert!((sf.care().weight_of("efficiency").unwrap() - original_weight).abs() < f64::EPSILON);
    }

    // --- Baseline and comparison tests ---

    #[test]
    fn record_baseline_captures_state() {
        let validator = EvolutionValidator::new(0.8);
        let sf = make_self_field();

        let baseline = validator.record_baseline(&sf);
        assert_eq!(baseline.care_weights.len(), 4);
        assert!(baseline.care_weights.iter().any(|(t, _)| t == "safety"));
        assert_eq!(baseline.boundary_rule_count, 0);
    }

    #[test]
    fn compare_no_change() {
        let validator = EvolutionValidator::new(0.8);
        let sf = make_self_field();

        let baseline = validator.record_baseline(&sf);
        let outcome = validator.compare_with_baseline(&baseline, &sf);
        assert_eq!(outcome, EvolutionOutcome::NoChange);
    }

    #[test]
    fn compare_improved_more_rules() {
        let validator = EvolutionValidator::new(0.8);
        let mut sf = make_self_field();

        let baseline = validator.record_baseline(&sf);

        // Add a rule
        sf.boundary_mut().add_rule(BoundaryRule {
            action_pattern: "test.*".to_string(),
            source_filter: None,
            action: BoundaryAction::Deny,
            risk_level: RiskLevel::Low,
            description: "test rule".to_string(),
            immutable: false,
        });

        let outcome = validator.compare_with_baseline(&baseline, &sf);
        assert_eq!(outcome, EvolutionOutcome::Improved);
    }

    #[test]
    fn compare_degraded_fewer_care_weight() {
        let validator = EvolutionValidator::new(0.8);
        let mut sf = make_self_field();

        let baseline = validator.record_baseline(&sf);

        // Decrease a care weight significantly
        sf.care_mut().adjust_weight("learning", -0.15);

        let outcome = validator.compare_with_baseline(&baseline, &sf);
        assert_eq!(outcome, EvolutionOutcome::Degraded);
    }

    // --- Rollback test ---

    #[test]
    fn rollback_restores_care_weights() {
        let validator = EvolutionValidator::new(0.8);
        let mut sf = make_self_field();

        let baseline = validator.record_baseline(&sf);

        // Modify care weights
        sf.care_mut().adjust_weight("efficiency", 0.3);
        let modified_weight = sf.care().weight_of("efficiency").unwrap();
        assert!((modified_weight - 0.8).abs() < f64::EPSILON);

        // Rollback
        validator.rollback(&baseline, &mut sf);
        let restored_weight = sf.care().weight_of("efficiency").unwrap();
        assert!((restored_weight - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn safety_weight_ever_at_floor() {
        // Verify that safety can never go below 0.8 via adjust_weight
        let sf = make_self_field();
        let result = sf.care().adjust_weight("safety", -1.0);
        assert_eq!(result, Some((1.0, 0.8)));
    }

    #[test]
    fn care_weight_clamped_minimum() {
        let sf = make_self_field();
        // learning starts at 0.3, adjust by -1.0 -> clamped to 0.1
        let result = sf.care().adjust_weight("learning", -1.0);
        assert_eq!(result, Some((0.3, 0.1)));
    }
}
