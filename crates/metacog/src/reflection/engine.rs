//! Reflection engine — transforms evidence-backed evaluations and problems
//! into structured reflection reports.

use std::collections::HashMap;

use thiserror::Error;

use super::model::{
    CausalHypothesis, ImprovementProposal, ProblemSummary, RecurringPattern, ReflectionInput,
    ReflectionReport,
};
use crate::improvement::{ProposalId, ProposalState};

#[derive(Debug, Error)]
pub enum ReflectionError {
    #[error("no problems provided for reflection")]
    NoProblems,
    #[error("reflection internal error: {0}")]
    Internal(String),
}

/// Reflection engine port — consumes evaluated experiences and problems, produces a report.
pub trait ReflectionEngine: Send + Sync {
    fn reflect(&self, input: ReflectionInput) -> Result<ReflectionReport, ReflectionError>;
}

/// A deterministic reflection engine that groups problems by category,
/// identifies recurring patterns, and produces observation recommendations.
///
/// Rules:
/// - A pattern is "recurring" when the same category appears with >= 2 confirmed/active problems.
/// - Strengths and weaknesses are derived from evaluation reports (passed vs failed).
/// - Causal hypotheses are simple co-occurrence-based (category + affected subject).
/// - No model-generated narrative, no floating-point scores.
pub struct DeterministicReflectionEngine;

impl ReflectionEngine for DeterministicReflectionEngine {
    fn reflect(&self, input: ReflectionInput) -> Result<ReflectionReport, ReflectionError> {
        if input.problems.is_empty() {
            return Err(ReflectionError::NoProblems);
        }

        // Determine experience range
        let experience_range = if input.experiences.is_empty() {
            "none".to_string()
        } else if input.experiences.len() == 1 {
            input.experiences[0].experience_id.0.clone()
        } else {
            format!(
                "{}-{}",
                input.experiences.first().unwrap().experience_id.0,
                input.experiences.last().unwrap().experience_id.0
            )
        };

        // Strengths and weaknesses from evaluations
        let mut strengths: Vec<String> = Vec::new();
        let mut weaknesses: Vec<String> = Vec::new();
        for eval in &input.evaluations {
            if eval.eligible {
                strengths.push(format!(
                    "evaluation {} (rubric {}) passed with weighted total {} millis",
                    eval.rubric.0,
                    eval.rubric_version,
                    eval.weighted_total_millis.unwrap_or(0)
                ));
            } else {
                let gate_failures: Vec<&str> = eval
                    .gates
                    .iter()
                    .filter(|g| !g.passed)
                    .map(|g| g.name.as_str())
                    .collect();
                weaknesses.push(format!(
                    "evaluation {} (rubric {}) failed gates: [{}]",
                    eval.rubric.0,
                    eval.rubric_version,
                    gate_failures.join(", ")
                ));
            }
        }

        // Group problems by category
        let mut by_category: HashMap<String, Vec<&ProblemSummary>> = HashMap::new();
        for problem in &input.problems {
            by_category
                .entry(problem.category.clone())
                .or_default()
                .push(problem);
        }

        // Identify recurring patterns: same category with >= 2 confirmed/active problems
        let mut recurring_patterns: Vec<RecurringPattern> = Vec::new();
        for (category, problems) in &by_category {
            let confirmed: Vec<&&ProblemSummary> = problems
                .iter()
                .filter(|p| p.state == "confirmed" || p.state == "active")
                .collect();
            if confirmed.len() >= 2 {
                let example_ids: Vec<String> =
                    confirmed.iter().map(|p| p.problem_id.clone()).collect();
                let contrary_evidence: Vec<String> = confirmed
                    .iter()
                    .filter(|p| p.has_contrary_evidence)
                    .flat_map(|p| p.contrary_evidence.iter().cloned())
                    .collect();
                recurring_patterns.push(RecurringPattern {
                    category: category.clone(),
                    occurrence_count: confirmed.len(),
                    example_ids,
                    contrary_evidence,
                });
            }
        }

        // Generate causal hypotheses from recurring patterns
        let mut causal_hypotheses: Vec<CausalHypothesis> = Vec::new();
        for pattern in &recurring_patterns {
            let mut supporting_ids: Vec<String> = pattern.example_ids.clone();
            supporting_ids.sort();
            supporting_ids.dedup();

            let mut contrary_ids: Vec<String> = pattern.contrary_evidence.clone();
            contrary_ids.sort();
            contrary_ids.dedup();

            // Confidence: higher for more occurrences, reduced by contrary
            let base_confidence = ((pattern.occurrence_count as u32).min(10) * 100) as u16;
            let contrary_penalty = (contrary_ids.len() as u16).saturating_mul(100);
            let confidence_millis = base_confidence.saturating_sub(contrary_penalty).max(100);

            causal_hypotheses.push(CausalHypothesis {
                description: format!(
                    "{} problems in category '{}' suggest a systematic issue with {} occurrences",
                    pattern.occurrence_count, pattern.category, pattern.occurrence_count
                ),
                confidence_millis,
                supporting_ids,
                contrary_ids,
            });
        }

        // Knowledge gaps: dimensions marked Unknown in evaluations
        let mut knowledge_gaps: Vec<String> = Vec::new();
        for eval in &input.evaluations {
            for dim in &eval.dimensions {
                if matches!(
                    dim.value,
                    fabric::types::metacognition_evaluation::DimensionValue::Unknown
                ) {
                    knowledge_gaps.push(format!(
                        "dimension '{}' is unknown in evaluation {} (rubric {})",
                        dim.name, eval.rubric.0, eval.rubric_version
                    ));
                }
            }
        }

        if knowledge_gaps.is_empty() {
            knowledge_gaps.push("no unknown dimensions detected".to_string());
        }

        // Recommended observations
        let mut recommended_observations: Vec<String> = Vec::new();
        for pattern in &recurring_patterns {
            recommended_observations.push(format!(
                "monitor category '{}' for at least 5 more occurrences to confirm pattern",
                pattern.category
            ));
        }
        if !causal_hypotheses.is_empty() {
            recommended_observations.push(
                "collect additional evidence for each causal hypothesis before proposing changes"
                    .to_string(),
            );
        }
        if recommended_observations.is_empty() {
            recommended_observations
                .push("no specific observation recommendations — continue monitoring".to_string());
        }

        let mut improvement_proposals: Vec<ImprovementProposal> = Vec::new();
        for pattern in &recurring_patterns {
            let sanitized = pattern
                .category
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                .collect::<String>();
            improvement_proposals.push(ImprovementProposal {
                id: ProposalId(format!("proposal-reflect-{sanitized}")),
                proposer: "metacog".into(),
                target_capability: format!("capability.{}", pattern.category),
                problem_ids: pattern.example_ids.clone(),
                proposed_change: format!(
                    "Address the recurring '{}' pattern through a governed candidate",
                    pattern.category
                ),
                expected_benefit: format!(
                    "Reduce measured '{}' problem occurrences",
                    pattern.category
                ),
                possible_regressions: vec![
                    "The candidate may reduce performance in unrelated dimensions".into(),
                ],
                validation_plan:
                    "Replay the same evaluation cohort and require all hard gates to pass".into(),
                rollback_plan: "Restore the baseline runtime and record the measured outcome"
                    .into(),
                authority_requirements: vec!["external-governor".into()],
                reversible: true,
                expires_at_ms: i64::MAX,
                state: ProposalState::Proposed,
            });
        }

        Ok(ReflectionReport {
            experience_range,
            strengths,
            weaknesses,
            recurring_patterns,
            causal_hypotheses,
            knowledge_gaps,
            recommended_observations,
            improvement_proposals,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_problem(
        id: &str,
        category: &str,
        state: &str,
        severity: u8,
        description: &str,
        has_contrary: bool,
        contrary: Vec<String>,
    ) -> ProblemSummary {
        ProblemSummary {
            problem_id: id.to_string(),
            category: category.to_string(),
            state: state.to_string(),
            severity_ordinal: severity,
            description: description.to_string(),
            has_contrary_evidence: has_contrary,
            contrary_evidence: contrary,
        }
    }

    #[test]
    fn empty_problems_returns_error() {
        let engine = DeterministicReflectionEngine;
        let input = ReflectionInput {
            experiences: vec![],
            evaluations: vec![],
            problems: vec![],
        };
        let result = engine.reflect(input);
        assert!(result.is_err());
    }

    #[test]
    fn two_confirmed_same_category_creates_recurring_pattern() {
        let engine = DeterministicReflectionEngine;
        let input = ReflectionInput {
            experiences: vec![],
            evaluations: vec![],
            problems: vec![
                make_problem(
                    "p1",
                    "timeout",
                    "confirmed",
                    3,
                    "req timeout",
                    false,
                    vec![],
                ),
                make_problem(
                    "p2",
                    "timeout",
                    "confirmed",
                    3,
                    "req timeout 2",
                    false,
                    vec![],
                ),
                make_problem(
                    "p3",
                    "correctness",
                    "confirmed",
                    2,
                    "wrong answer",
                    false,
                    vec![],
                ),
            ],
        };
        let report = engine.reflect(input).unwrap();

        assert_eq!(report.recurring_patterns.len(), 1);
        let pattern = &report.recurring_patterns[0];
        assert_eq!(pattern.category, "timeout");
        assert_eq!(pattern.occurrence_count, 2);
        assert!(pattern.example_ids.contains(&"p1".to_string()));
        assert!(pattern.example_ids.contains(&"p2".to_string()));
        assert!(pattern.contrary_evidence.is_empty());
    }

    #[test]
    fn contrary_evidence_is_preserved_in_pattern() {
        let engine = DeterministicReflectionEngine;
        let input = ReflectionInput {
            experiences: vec![],
            evaluations: vec![],
            problems: vec![
                make_problem(
                    "p1",
                    "timeout",
                    "confirmed",
                    3,
                    "req timeout",
                    true,
                    vec!["sometimes succeeds under light load".to_string()],
                ),
                make_problem(
                    "p2",
                    "timeout",
                    "confirmed",
                    3,
                    "req timeout 2",
                    false,
                    vec![],
                ),
            ],
        };
        let report = engine.reflect(input).unwrap();

        assert_eq!(report.recurring_patterns.len(), 1);
        let pattern = &report.recurring_patterns[0];
        assert!(!pattern.contrary_evidence.is_empty());
        assert!(pattern
            .contrary_evidence
            .contains(&"sometimes succeeds under light load".to_string()));
    }

    #[test]
    fn observation_recommendations_are_produced() {
        let engine = DeterministicReflectionEngine;
        let input = ReflectionInput {
            experiences: vec![],
            evaluations: vec![],
            problems: vec![
                make_problem("p1", "timeout", "confirmed", 3, "timeout", false, vec![]),
                make_problem("p2", "timeout", "confirmed", 3, "timeout 2", false, vec![]),
            ],
        };
        let report = engine.reflect(input).unwrap();

        assert!(!report.recommended_observations.is_empty());
        let has_monitor = report
            .recommended_observations
            .iter()
            .any(|r| r.contains("monitor category 'timeout'"));
        assert!(has_monitor);
    }

    #[test]
    fn single_confirmed_does_not_create_recurring_pattern() {
        let engine = DeterministicReflectionEngine;
        let input = ReflectionInput {
            experiences: vec![],
            evaluations: vec![],
            problems: vec![
                make_problem(
                    "p1",
                    "timeout",
                    "confirmed",
                    3,
                    "req timeout",
                    false,
                    vec![],
                ),
                make_problem(
                    "p2",
                    "correctness",
                    "confirmed",
                    2,
                    "wrong answer",
                    false,
                    vec![],
                ),
            ],
        };
        let report = engine.reflect(input).unwrap();
        assert!(report.recurring_patterns.is_empty());
    }

    #[test]
    fn non_confirmed_problems_are_not_counted_as_recurring() {
        let engine = DeterministicReflectionEngine;
        let input = ReflectionInput {
            experiences: vec![],
            evaluations: vec![],
            problems: vec![
                make_problem("p1", "timeout", "observed", 3, "timeout", false, vec![]),
                make_problem("p2", "timeout", "observed", 3, "timeout 2", false, vec![]),
                make_problem("p3", "timeout", "disputed", 3, "timeout 3", false, vec![]),
            ],
        };
        let report = engine.reflect(input).unwrap();
        assert!(report.recurring_patterns.is_empty());
    }

    #[test]
    fn improvement_proposals_are_governable_and_unapproved() {
        let engine = DeterministicReflectionEngine;
        let input = ReflectionInput {
            experiences: vec![],
            evaluations: vec![],
            problems: vec![
                make_problem("p1", "timeout", "confirmed", 3, "timeout", false, vec![]),
                make_problem("p2", "timeout", "confirmed", 3, "timeout 2", false, vec![]),
            ],
        };
        let report = engine.reflect(input).unwrap();

        assert!(!report.improvement_proposals.is_empty());
        for proposal in &report.improvement_proposals {
            assert!(proposal.id.0.contains("proposal-reflect-"));
            assert_eq!(proposal.state, ProposalState::Proposed);
            assert!(!proposal.problem_ids.is_empty());
            assert!(!proposal.validation_plan.is_empty());
            assert!(!proposal.rollback_plan.is_empty());
        }
    }
}
