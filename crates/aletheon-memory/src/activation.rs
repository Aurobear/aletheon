/// Compute memory activation score using an ACT-R-inspired formula.
///
/// Higher activation = more likely to be retrieved.
/// Components:
/// - base importance (caller-assigned, 0.0-1.0)
/// - recency (decays with time since last access)
/// - frequency (logarithmic scaling of access count)
pub fn compute_activation(entry: &ActivationEntry, now_timestamp: i64) -> f64 {
    let base = entry.importance.clamp(0.0, 1.0);

    // Recency: time since last access in hours
    let age_hours = ((now_timestamp - entry.last_accessed_at).max(0)) as f64 / 3600.0;
    // Recency decays as 1/(1 + sqrt(age_hours))
    let recency = 1.0 / (1.0 + age_hours.sqrt());

    // Frequency: logarithmic scaling
    let frequency = (entry.access_count as f64 + 1.0).ln() / 5.0_f64.ln(); // normalized by ln(5)
    let frequency = frequency.min(1.0);

    base * 0.4 + recency * 0.35 + frequency * 0.25
}

/// Input for activation computation.
/// Separated from MemoryEntry to allow flexibility.
#[derive(Debug, Clone)]
pub struct ActivationEntry {
    pub importance: f64,
    pub access_count: i64,
    pub last_accessed_at: i64, // unix timestamp in seconds
}

impl ActivationEntry {
    pub fn new(importance: f64, access_count: i64, last_accessed_at: i64) -> Self {
        Self {
            importance,
            access_count,
            last_accessed_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> i64 {
        1700000000
    } // fixed timestamp

    #[test]
    fn fresh_high_importance_high_activation() {
        let entry = ActivationEntry::new(0.9, 10, now());
        let score = compute_activation(&entry, now());
        assert!(
            score > 0.7,
            "fresh high-importance entry should have high activation: {}",
            score
        );
    }

    #[test]
    fn old_entry_lower_activation() {
        let entry = ActivationEntry::new(0.9, 10, now() - 7 * 24 * 3600); // 7 days ago
        let score_old = compute_activation(&entry, now());
        let entry_fresh = ActivationEntry::new(0.9, 10, now());
        let score_fresh = compute_activation(&entry_fresh, now());
        assert!(score_fresh > score_old);
    }

    #[test]
    fn frequent_access_boosts() {
        let entry_low = ActivationEntry::new(0.5, 1, now());
        let entry_high = ActivationEntry::new(0.5, 20, now());
        let score_low = compute_activation(&entry_low, now());
        let score_high = compute_activation(&entry_high, now());
        assert!(score_high > score_low);
    }

    #[test]
    fn zero_access_count_works() {
        let entry = ActivationEntry::new(0.5, 0, now());
        let score = compute_activation(&entry, now());
        assert!(score > 0.0);
    }

    #[test]
    fn importance_dominates_base() {
        let entry_low = ActivationEntry::new(0.1, 1, now());
        let entry_high = ActivationEntry::new(0.9, 1, now());
        let score_low = compute_activation(&entry_low, now());
        let score_high = compute_activation(&entry_high, now());
        assert!(score_high > score_low);
    }

    #[test]
    fn bounded_0_to_1() {
        let entry = ActivationEntry::new(1.0, 100, now());
        let score = compute_activation(&entry, now());
        assert!(score >= 0.0 && score <= 1.0, "score out of range: {}", score);
    }

    #[test]
    fn very_old_entry_much_lower_than_fresh() {
        let entry = ActivationEntry::new(0.5, 1, now() - 365 * 24 * 3600); // 1 year ago
        let score_old = compute_activation(&entry, now());
        let entry_fresh = ActivationEntry::new(0.5, 1, now());
        let score_fresh = compute_activation(&entry_fresh, now());
        // With importance=0.5, base alone contributes 0.2, so score can't be below 0.2.
        // But the very old entry should be significantly lower than the fresh one.
        assert!(
            score_old < score_fresh * 0.7,
            "very old entry ({}) should be much lower than fresh ({})",
            score_old,
            score_fresh
        );
    }

    #[test]
    fn recency_matters_more_than_frequency() {
        // A fresh entry with low access should beat an old entry with high access
        let fresh = ActivationEntry::new(0.5, 1, now());
        let old_frequent = ActivationEntry::new(0.5, 50, now() - 30 * 24 * 3600);
        let score_fresh = compute_activation(&fresh, now());
        let score_old = compute_activation(&old_frequent, now());
        assert!(score_fresh > score_old);
    }
}
