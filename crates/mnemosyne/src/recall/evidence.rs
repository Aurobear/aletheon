use serde::{Deserialize, Serialize};

/// Evidence level stamped on each recall result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceLevel {
    /// Exact entity/alias match (score >= 0.95).
    AliasHit,
    /// Title-bearer match (score >= 0.90).
    ExactTitleMatch,
    /// Above HIGH_MATCH_FLOOR (0.85).
    HighVectorMatch,
    /// Above SOLID_MATCH_FLOOR (0.6).
    KeywordExact,
    /// Everything else.
    WeakSemantic,
}

/// Create-safety hint derived from evidence level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateSafety {
    /// Strong evidence this IS the thing — do NOT create a duplicate.
    Exists,
    /// Likely the page — prefer updating over creating.
    Probable,
    /// No strong evidence — look closer.
    Unknown,
}

impl EvidenceLevel {
    pub fn create_safety(&self) -> CreateSafety {
        match self {
            EvidenceLevel::AliasHit
            | EvidenceLevel::ExactTitleMatch
            | EvidenceLevel::HighVectorMatch => CreateSafety::Exists,
            EvidenceLevel::KeywordExact => CreateSafety::Probable,
            EvidenceLevel::WeakSemantic => CreateSafety::Unknown,
        }
    }
}

/// Stamp an evidence level on a recall result based on its score.
/// Pure, idempotent function.
pub fn stamp_evidence(score: f32) -> Option<EvidenceLevel> {
    if !score.is_finite() {
        return None;
    }
    if score >= 0.95 {
        Some(EvidenceLevel::AliasHit)
    } else if score >= 0.90 {
        Some(EvidenceLevel::ExactTitleMatch)
    } else if score >= 0.85 {
        Some(EvidenceLevel::HighVectorMatch)
    } else if score >= 0.60 {
        Some(EvidenceLevel::KeywordExact)
    } else {
        Some(EvidenceLevel::WeakSemantic)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamp_evidence_returns_correct_levels() {
        assert_eq!(stamp_evidence(0.96), Some(EvidenceLevel::AliasHit));
        assert_eq!(stamp_evidence(0.90), Some(EvidenceLevel::ExactTitleMatch));
        assert_eq!(stamp_evidence(0.85), Some(EvidenceLevel::HighVectorMatch));
        assert_eq!(stamp_evidence(0.60), Some(EvidenceLevel::KeywordExact));
        assert_eq!(stamp_evidence(0.30), Some(EvidenceLevel::WeakSemantic));
    }

    #[test]
    fn stamp_evidence_returns_none_for_nan() {
        assert_eq!(stamp_evidence(f32::NAN), None);
    }

    #[test]
    fn create_safety_maps_correctly() {
        assert_eq!(
            EvidenceLevel::AliasHit.create_safety(),
            CreateSafety::Exists
        );
        assert_eq!(
            EvidenceLevel::KeywordExact.create_safety(),
            CreateSafety::Probable
        );
        assert_eq!(
            EvidenceLevel::WeakSemantic.create_safety(),
            CreateSafety::Unknown
        );
    }
}
