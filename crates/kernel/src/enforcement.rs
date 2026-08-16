//! K6 Kernel enforcement seam gates (Agent Kernel V2).
//!
//! Delivers the stable per-family registration gate and the writer/executor
//! uniqueness gate that the E-series joint cutovers (E2-K6a/E4-K6b/E5-K6c/
//! E6-K6d) consume.  This module is **gate/check only** — it does not migrate
//! any extension.  A failed shard returns to legacy-authoritative; Kernel
//! receipts never replace domain-owned Hardware veto / Gmail outbox / Pi
//! terminal evidence.

use serde::{Deserialize, Serialize};

/// Extension enforcement family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EnforcementFamily {
    Gmail,
    Hardware,
    RobotVla,
    Pi,
}

/// A per-family registration gate decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FamilyGate {
    /// The family's owner PR (E2-K6a / E4-K6b / E5-K6c / E6-K6d) is the sole
    /// executor/writer; Kernel only provides the seam.
    OwnerCutoverRequired(EnforcementFamily),
    /// The family is not yet switched; keep it legacy-authoritative.
    LegacyAuthoritative,
}

/// The writer/executor uniqueness gate: only one production writer/executor
/// may be active per operation generation.  A shard that fails a check must
/// return to legacy-authoritative — it must not pass with the feature
/// disabled.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UniquenessViolation {
    #[error("duplicate executor registered for family {family:?}")]
    DuplicateExecutor { family: EnforcementFamily },
    #[error("duplicate writer registered for family {family:?}")]
    DuplicateWriter { family: EnforcementFamily },
    #[error("shadow performed a real effect (forbidden)")]
    ShadowEffect,
}

/// The K6 enforcement seam gate.  Composition root consults it at bootstrap;
/// it never drives an extension itself.
#[derive(Debug, Clone)]
pub struct EnforcementSeamGate {
    writers: std::collections::HashMap<EnforcementFamily, u32>,
    executors: std::collections::HashMap<EnforcementFamily, u32>,
}

impl Default for EnforcementSeamGate {
    fn default() -> Self {
        Self::new()
    }
}

impl EnforcementSeamGate {
    pub fn new() -> Self {
        Self {
            writers: std::collections::HashMap::new(),
            executors: std::collections::HashMap::new(),
        }
    }

    /// Register a writer for a family.  A second writer for the same family is
    /// a uniqueness violation.
    pub fn register_writer(
        &mut self,
        family: EnforcementFamily,
    ) -> Result<(), UniquenessViolation> {
        let n = self.writers.entry(family).or_insert(0);
        if *n > 0 {
            return Err(UniquenessViolation::DuplicateWriter { family });
        }
        *n += 1;
        Ok(())
    }

    /// Register an executor for a family.  A second executor for the same
    /// family is a uniqueness violation.
    pub fn register_executor(
        &mut self,
        family: EnforcementFamily,
    ) -> Result<(), UniquenessViolation> {
        let n = self.executors.entry(family).or_insert(0);
        if *n > 0 {
            return Err(UniquenessViolation::DuplicateExecutor { family });
        }
        *n += 1;
        Ok(())
    }

    /// The per-family decision: owner PR is the sole cutover; Kernel only
    /// provides the seam.
    pub fn family_decision(&self, family: EnforcementFamily) -> FamilyGate {
        FamilyGate::OwnerCutoverRequired(family)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_writer_is_rejected() {
        let mut gate = EnforcementSeamGate::new();
        gate.register_writer(EnforcementFamily::Pi).unwrap();
        assert_eq!(
            gate.register_writer(EnforcementFamily::Pi),
            Err(UniquenessViolation::DuplicateWriter {
                family: EnforcementFamily::Pi
            })
        );
    }

    #[test]
    fn family_decision_requires_owner_cutover() {
        let gate = EnforcementSeamGate::new();
        assert_eq!(
            gate.family_decision(EnforcementFamily::Gmail),
            FamilyGate::OwnerCutoverRequired(EnforcementFamily::Gmail)
        );
    }
}
