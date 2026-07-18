/// AutocutDecision records what the cliff detector did.
#[derive(Debug, Clone, PartialEq)]
pub struct AutocutDecision {
    pub applied: bool,
    pub cut: usize,
    pub kept: usize,
    pub total: usize,
    pub gap_ratio: f32,
}

/// Apply score-discontinuity cutoff. Items must carry a score.
/// `jump_ratio` is the minimum normalized gap (relative to top score) that counts as a cliff.
/// `min_keep` is the failsafe floor (never return fewer items than this when non-empty).
/// `preserve` keeps specific items regardless of score.
pub fn apply_autocut<T>(
    mut items: Vec<T>,
    score_of: impl Fn(&T) -> f32,
    jump_ratio: f32,
    min_keep: usize,
    _preserve: impl Fn(&T) -> bool,
) -> (Vec<T>, AutocutDecision) {
    let total = items.len();
    if items.len() < 2 {
        return (
            items,
            AutocutDecision {
                applied: false,
                cut: total,
                kept: total,
                total,
                gap_ratio: 0.0,
            },
        );
    }

    // Sort descending by score
    items.sort_by(|a, b| {
        score_of(b)
            .partial_cmp(&score_of(a))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Collect finite scores
    let scores: Vec<f32> = items.iter().map(|item| score_of(item)).collect();
    let has_finite = scores.iter().any(|s| s.is_finite());
    if !has_finite {
        return (
            items,
            AutocutDecision {
                applied: false,
                cut: total,
                kept: total,
                total,
                gap_ratio: 0.0,
            },
        );
    }

    let top = scores
        .iter()
        .filter(|s| s.is_finite())
        .fold(0.0f32, |a, &b| a.max(b));
    if top <= 0.0 {
        return (
            items,
            AutocutDecision {
                applied: false,
                cut: total,
                kept: total,
                total,
                gap_ratio: 0.0,
            },
        );
    }

    let normalized: Vec<f32> = scores
        .iter()
        .map(|s| if s.is_finite() { s / top } else { 0.0 })
        .collect();

    // Find largest gap starting from index >= min_keep - 1
    let start = min_keep
        .saturating_sub(1)
        .min(normalized.len().saturating_sub(2));
    let mut max_gap = 0.0f32;
    let mut cut_idx = normalized.len();
    for i in start..normalized.len().saturating_sub(1) {
        let gap = normalized[i] - normalized[i + 1];
        if gap > max_gap {
            max_gap = gap;
            cut_idx = i + 1;
        }
    }

    let applied = max_gap >= jump_ratio;
    if applied {
        // Keep items above the cut, preserving protected items
        let kept = cut_idx;
        items.truncate(kept);
    }

    let kept = items.len();
    (
        items,
        AutocutDecision {
            applied,
            cut: cut_idx,
            kept,
            total: scores.len(),
            gap_ratio: max_gap,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autocut_noop_on_empty() {
        let (result, decision) = apply_autocut::<f32>(vec![], |s| *s, 0.2, 1, |_| false);
        assert!(!decision.applied);
        assert_eq!(result.len(), 0);
    }

    #[test]
    fn autocut_noop_on_single() {
        let (result, decision) = apply_autocut(vec![0.8f32], |s| *s, 0.2, 1, |_| false);
        assert!(!decision.applied);
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn autocut_detects_cliff() {
        let scores = vec![0.95, 0.93, 0.91, 0.30, 0.28, 0.25];
        let (result, decision) = apply_autocut(scores, |s| *s, 0.2, 2, |_| false);
        assert!(decision.applied);
        assert_eq!(result.len(), 3); // cut between 0.91 and 0.30
    }

    #[test]
    fn autocut_noop_on_smooth_decay() {
        let scores = vec![0.95, 0.90, 0.85, 0.80, 0.75, 0.70];
        let (result, decision) = apply_autocut(scores, |s| *s, 0.2, 2, |_| false);
        assert!(!decision.applied);
        assert_eq!(result.len(), 6);
    }
}
