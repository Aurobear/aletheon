/// Memory decay based on Ebbinghaus forgetting curve.
///
/// Memories lose "strength" over time unless accessed.
/// Accessed memories get a strength boost (re-consolidation).
/// Memories below a threshold are candidates for forgetting.
///
/// Compute current strength of a memory entry.
///
/// Uses a simplified Ebbinghaus model:
///   strength = base_strength * e^(-decay_rate * days_since_access)
///
/// When accessed, strength is boosted toward 1.0:
///   new_strength = old_strength + (1.0 - old_strength) * 0.3
pub fn compute_strength(
    base_strength: f64,
    decay_rate: f64,
    last_accessed_timestamp: i64,
    now_timestamp: i64,
) -> f64 {
    let days = ((now_timestamp - last_accessed_timestamp).max(0)) as f64 / 86400.0;
    let strength = base_strength * (-decay_rate * days).exp();
    strength.clamp(0.0, 1.0)
}

/// Apply a memory access event: boost strength toward 1.0.
pub fn apply_access_boost(current_strength: f64) -> f64 {
    let boosted = current_strength + (1.0 - current_strength) * 0.3;
    boosted.clamp(0.0, 1.0)
}

/// Check if a memory should be forgotten (strength below threshold).
pub fn should_forget(strength: f64, threshold: f64) -> bool {
    strength < threshold
}

/// Default decay rate: strength halves every 7 days.
pub const DEFAULT_DECAY_RATE: f64 = 0.099; // ln(2) / 7 ≈ 0.099

/// Minimum importance threshold: memories with importance > 0.9 never auto-forget.
pub const IMMORTAL_IMPORTANCE: f64 = 0.9;

/// Default forgetting threshold: memories with strength < 0.1 are candidates for forgetting.
pub const DEFAULT_FORGET_THRESHOLD: f64 = 0.1;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_memory_full_strength() {
        let now = 1700000000;
        let strength = compute_strength(1.0, DEFAULT_DECAY_RATE, now, now);
        assert!((strength - 1.0).abs() < 0.001);
    }

    #[test]
    fn decay_after_one_week() {
        let now = 1700000000;
        let week_ago = now - 7 * 86400;
        let strength = compute_strength(1.0, DEFAULT_DECAY_RATE, week_ago, now);
        // Should be approximately 0.5 (half-life = 7 days)
        assert!(
            (strength - 0.5).abs() < 0.05,
            "strength after 1 week: {}",
            strength
        );
    }

    #[test]
    fn decay_after_one_month() {
        let now = 1700000000;
        let month_ago = now - 30 * 86400;
        let strength = compute_strength(1.0, DEFAULT_DECAY_RATE, month_ago, now);
        assert!(strength < 0.1, "strength after 1 month: {}", strength);
    }

    #[test]
    fn access_boost_works() {
        let boosted = apply_access_boost(0.3);
        // 0.3 + (1.0 - 0.3) * 0.3 = 0.3 + 0.21 = 0.51
        assert!((boosted - 0.51).abs() < 0.01);
    }

    #[test]
    fn access_boost_at_high_strength() {
        let boosted = apply_access_boost(0.9);
        // 0.9 + (1.0 - 0.9) * 0.3 = 0.9 + 0.03 = 0.93
        assert!((boosted - 0.93).abs() < 0.01);
    }

    #[test]
    fn should_forget_below_threshold() {
        assert!(should_forget(0.05, DEFAULT_FORGET_THRESHOLD));
        assert!(!should_forget(0.5, DEFAULT_FORGET_THRESHOLD));
    }

    #[test]
    fn zero_decay_rate_preserves() {
        let now = 1700000000;
        let year_ago = now - 365 * 86400;
        let strength = compute_strength(1.0, 0.0, year_ago, now);
        assert!((strength - 1.0).abs() < 0.001);
    }

    #[test]
    fn high_decay_rate_forgets_fast() {
        let now = 1700000000;
        let day_ago = now - 86400;
        let strength = compute_strength(1.0, 1.0, day_ago, now); // decay_rate=1.0: forgets in ~1 day
        assert!(strength < 0.5);
    }

    #[test]
    fn strength_always_bounded() {
        let now = 1700000000;
        // Even with extreme inputs
        let s1 = compute_strength(100.0, 100.0, now - 100 * 86400, now);
        assert!(s1 >= 0.0 && s1 <= 1.0);
        let s2 = compute_strength(0.0, 0.0, now, now);
        assert!(s2 >= 0.0 && s2 <= 1.0);
    }
}
