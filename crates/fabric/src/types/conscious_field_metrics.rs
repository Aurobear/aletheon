//! Bounded, deterministic field metrics for conscious-core acceptance evidence.
//!
//! This module provides snapshots, history, and computable indicators that
//! satisfy the R3 field-evidence requirements (AC-F.1, AC-F.2).
//!
//! The module owns no clock/I/O.  All computation is deterministic and
//! based only on the bounded numeric state pushed by the coordinator.
//!
//! ## Quantization
//!
//! Values in [0,1] are quantized into 16 bins via:
//! ```text
//! min(floor(value * 16.0), 15)
//! ```
//!
//! ## Mutual information
//!
//! Empirical lagged mutual information is computed over the 64-snapshot
//! window with a small epsilon (1e-4) to avoid division by zero:
//! ```text
//! I(S_t; S_{t+k}) = sum pxy * ln(pxy / (px * py + epsilon))
//! ```

use std::collections::{HashMap, VecDeque};

use serde::{Deserialize, Serialize};

use crate::dasein::CareActionKind;

/// Maximum number of snapshots retained in history.
pub const MAX_FIELD_METRIC_HISTORY: usize = 64;

/// Number of trailing snapshots required for convergence check.
pub const QUIET_CONVERGENCE_WINDOW: usize = 8;

/// Number of quantization bins for values in [0, 1].
pub const QUANTIZATION_BINS: usize = 16;

/// Small constant to avoid division-by-zero in mutual information.
pub const MI_EPSILON: f64 = 1e-4;

// ---------------------------------------------------------------------------
// Snapshot
// ---------------------------------------------------------------------------

/// A single bounded, numeric snapshot of conscious field state.
///
/// Contains only causal identifiers and measurements. No prompts,
/// tool inputs, secrets, or hidden reasoning.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldMetricSnapshot {
    /// Broadcast epoch this snapshot corresponds to.
    pub broadcast_epoch: u64,
    /// Dasein self version at time of snapshot.
    pub dasein_version: u64,
    /// The eight salience dimensions in order:
    /// urgency, goal_relevance, self_relevance, novelty, confidence,
    /// prediction_error, affect_intensity, social_relevance.
    pub salience: [f64; 8],
    /// Selected care action, if any.
    pub care_action: Option<CareActionKind>,
    /// Concern urgency at time of snapshot.
    pub concern_urgency: f64,
    /// L1 update delta from the preceding snapshot (0 for the first).
    pub update_delta: f64,
    /// Prior protention/prediction salience direction (8-d), or zeros.
    pub protention_salience: [f64; 8],
    /// Identifier for the trace event that this snapshot links to.
    pub trace_event_id: String,
}

impl FieldMetricSnapshot {
    /// Create a zero-valued placeholder snapshot.
    pub fn zero() -> Self {
        Self {
            broadcast_epoch: 0,
            dasein_version: 0,
            salience: [0.0; 8],
            care_action: None,
            concern_urgency: 0.0,
            update_delta: 0.0,
            protention_salience: [0.0; 8],
            trace_event_id: String::new(),
        }
    }

    /// Compute the L1 delta of the 8-d salience vector between two snapshots.
    pub fn salience_l1_delta(prev: &[f64; 8], cur: &[f64; 8]) -> f64 {
        prev.iter()
            .zip(cur.iter())
            .map(|(a, b)| (b - a).abs())
            .sum()
    }

    /// Return true when all eight salience dimensions are finite and in [0, 1].
    pub fn is_bounded(&self) -> bool {
        self.salience.iter().all(|v| v.is_finite() && *v >= 0.0 && *v <= 1.0)
    }
}

// ---------------------------------------------------------------------------
// Quantization helper
// ---------------------------------------------------------------------------

/// Quantize a value in [0, 1] into an integer bin in [0, QUANTIZATION_BINS - 1].
///
/// Out-of-range values are clamped.
#[inline]
pub fn quantize(value: f64) -> usize {
    let raw = (value.clamp(0.0, 1.0) * QUANTIZATION_BINS as f64).floor() as usize;
    raw.min(QUANTIZATION_BINS - 1)
}

// ---------------------------------------------------------------------------
// Indicators
// ---------------------------------------------------------------------------

/// Computed indicators derived from a [`FieldMetricHistory`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldMetricIndicators {
    /// True when the last QUIET_CONVERGENCE_WINDOW samples show non-increasing
    /// L1 deltas within epsilon, and the final delta is below the convergence
    /// threshold.
    pub attractor_converged: bool,
    /// Lagged mutual information (lag=1) over the salience dimension `urgency`
    /// across the history window.
    pub lagged_mutual_information: Option<f64>,
    /// Mean of the non-zero update deltas in the window, or None if no updates.
    pub update_delta_mean: Option<f64>,
    /// Cosine alignment between the last protention salience direction and the
    /// last action salience direction, or None when either vector has zero norm.
    pub cos_alignment: Option<f64>,
}

// ---------------------------------------------------------------------------
// History
// ---------------------------------------------------------------------------

/// Bounded, ring-buffer history of [`FieldMetricSnapshot`] entries.
///
/// Maximum capacity is [`MAX_FIELD_METRIC_HISTORY`] (64). When full, the
/// oldest entry is evicted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldMetricHistory {
    entries: VecDeque<FieldMetricSnapshot>,
}

impl Default for FieldMetricHistory {
    fn default() -> Self {
        Self {
            entries: VecDeque::with_capacity(MAX_FIELD_METRIC_HISTORY),
        }
    }
}

impl FieldMetricHistory {
    /// Build a history from an ordered list of snapshots.
    ///
    /// If more than 64 snapshots are provided, only the last 64 are retained.
    /// Returns an error if any snapshot has unbounded (non-finite) salience.
    pub fn from_snapshots(
        snapshots: impl IntoIterator<Item = FieldMetricSnapshot>,
    ) -> anyhow::Result<Self> {
        let mut history = Self::default();
        for snap in snapshots {
            history.push(snap)?;
        }
        Ok(history)
    }

    /// Push a new snapshot.
    ///
    /// The `update_delta` field is computed as the L1 salience delta from the
    /// most recent entry, or 0.0 if this is the first entry.
    ///
    /// Returns an error if any salience value is not finite.
    pub fn push(&mut self, mut snapshot: FieldMetricSnapshot) -> anyhow::Result<()> {
        anyhow::ensure!(
            snapshot.salience.iter().all(|v| v.is_finite()),
            "salience values must be finite"
        );

        // Compute update delta from previous snapshot.
        let prev_salience = self.entries.back().map(|s| &s.salience);
        snapshot.update_delta = match prev_salience {
            Some(prev) => FieldMetricSnapshot::salience_l1_delta(prev, &snapshot.salience),
            None => 0.0,
        };

        if self.entries.len() >= MAX_FIELD_METRIC_HISTORY {
            self.entries.pop_front();
        }
        self.entries.push_back(snapshot);
        Ok(())
    }

    /// Number of entries currently stored.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when no entries have been pushed.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Access the raw snapshot entries.
    pub fn entries(&self) -> &VecDeque<FieldMetricSnapshot> {
        &self.entries
    }

    // ------------------------------------------------------------------
    // Computed indicators
    // ------------------------------------------------------------------

    /// Compute all indicators for the current history.
    pub fn indicators(&self) -> FieldMetricIndicators {
        FieldMetricIndicators {
            attractor_converged: self.check_attractor_converged(),
            lagged_mutual_information: self.lagged_mutual_information(1),
            update_delta_mean: self.update_delta_mean(),
            cos_alignment: self.cos_alignment(),
        }
    }

    /// Check whether the attractor is converged: the last
    /// [`QUIET_CONVERGENCE_WINDOW`] deltas must be non-increasing
    /// (within epsilon) and the final delta must be below epsilon.
    fn check_attractor_converged(&self) -> bool {
        if self.entries.len() < QUIET_CONVERGENCE_WINDOW {
            return false;
        }

        let deltas: Vec<f64> = self
            .entries
            .iter()
            .rev()
            .take(QUIET_CONVERGENCE_WINDOW)
            .map(|s| s.update_delta)
            .collect();

        // Check non-increasing: each delta must be >= the next (earlier)
        // or within epsilon tolerance.
        for pair in deltas.windows(2) {
            if pair[0] > pair[1] + MI_EPSILON {
                return false;
            }
        }

        // Final (oldest in window) delta must be below epsilon.
        deltas[QUIET_CONVERGENCE_WINDOW - 1] <= MI_EPSILON
    }

    /// Lagged mutual information for a single salience dimension `urgency`
    /// at lag `k`.
    ///
    /// Returns `None` if the history has fewer than 2 entries or if the
    /// denominator is zero for all bin pairs.
    pub fn lagged_mutual_information(&self, lag: usize) -> Option<f64> {
        let n = self.entries.len();
        if n < 2 || lag == 0 || lag >= n {
            return None;
        }

        // Collect pairs (S_t, S_{t+k}) for the urgency dimension only.
        let pairs: Vec<(usize, usize)> = (0..n - lag)
            .map(|t| {
                let xt = quantize(self.entries[t].salience[0]);
                let xtk = quantize(self.entries[t + lag].salience[0]);
                (xt, xtk)
            })
            .collect();

        let total = pairs.len() as f64;

        // Marginal counts.
        let mut px_counts: [usize; QUANTIZATION_BINS] = [0; QUANTIZATION_BINS];
        let mut py_counts: [usize; QUANTIZATION_BINS] = [0; QUANTIZATION_BINS];
        let mut pxy_counts: HashMap<(usize, usize), usize> = HashMap::new();

        for &(x, y) in &pairs {
            px_counts[x] += 1;
            py_counts[y] += 1;
            *pxy_counts.entry((x, y)).or_insert(0) += 1;
        }

        let mut mi = 0.0_f64;

        for (&(x, y), &count) in &pxy_counts {
            let pxy = count as f64 / total;
            let px = px_counts[x] as f64 / total;
            let py = py_counts[y] as f64 / total;

            let denom = px * py;
            if denom <= 0.0 {
                continue;
            }

            let ratio = pxy / (denom + MI_EPSILON);
            if ratio > 0.0 {
                mi += pxy * ratio.ln();
            }
        }

        Some(mi)
    }

    /// Mean of the non-zero update deltas in the history.
    ///
    /// Returns `None` when no non-zero deltas exist.
    pub fn update_delta_mean(&self) -> Option<f64> {
        let non_zero: Vec<f64> = self
            .entries
            .iter()
            .map(|s| s.update_delta)
            .filter(|&d| d > 0.0)
            .collect();

        if non_zero.is_empty() {
            return None;
        }

        let sum: f64 = non_zero.iter().sum();
        Some(sum / non_zero.len() as f64)
    }

    /// Cosine alignment between the last protention salience direction and
    /// the last action salience direction.
    ///
    /// Returns `None` when either vector has zero norm or fewer than 1 entry.
    pub fn cos_alignment(&self) -> Option<f64> {
        let last = self.entries.back()?;

        let dot: f64 = last
            .protention_salience
            .iter()
            .zip(last.salience.iter())
            .map(|(p, a)| p * a)
            .sum();

        let norm_p: f64 = last
            .protention_salience
            .iter()
            .map(|v| v * v)
            .sum::<f64>()
            .sqrt();

        let norm_a: f64 = last
            .salience
            .iter()
            .map(|v| v * v)
            .sum::<f64>()
            .sqrt();

        if norm_p == 0.0 || norm_a == 0.0 {
            return None;
        }

        Some(dot / (norm_p * norm_a))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- helpers ----

    fn snapshot_with_urgency(u: f64) -> FieldMetricSnapshot {
        let mut s = FieldMetricSnapshot::zero();
        s.salience[0] = u.clamp(0.0, 1.0);
        s.trace_event_id = format!("snap-{:.2}", u);
        s
    }

    fn continuous_fixture() -> Vec<FieldMetricSnapshot> {
        // Slowly increasing urgency, staying in similar bins.
        let mut out = Vec::new();
        for i in 0..64 {
            let u = 0.1 + (i as f64) * 0.01; // 0.10 .. 0.73
            let mut s = FieldMetricSnapshot::zero();
            s.broadcast_epoch = i;
            s.dasein_version = 1;
            s.salience[0] = u.clamp(0.0, 1.0);
            s.trace_event_id = format!("cont-{}", i);
            out.push(s);
        }
        out
    }

    fn reset_fixture() -> Vec<FieldMetricSnapshot> {
        // Reset at entry 32: urgency jumps back to 0.1.
        let mut out = Vec::new();
        for i in 0..64 {
            let u = if i < 32 {
                0.1 + (i as f64) * 0.01
            } else {
                0.1 + ((i - 32) as f64) * 0.01
            };
            let mut s = FieldMetricSnapshot::zero();
            s.broadcast_epoch = i;
            s.dasein_version = 1;
            s.salience[0] = u.clamp(0.0, 1.0);
            s.trace_event_id = format!("reset-{}", i);
            out.push(s);
        }
        out
    }

    fn quiet_snapshot(_epoch: u64) -> FieldMetricSnapshot {
        // Zero salience, zero delta.
        let mut s = FieldMetricSnapshot::zero();
        s.trace_event_id = format!("quiet-{}", _epoch);
        s
    }

    // ---- quantization ----

    #[test]
    fn quantize_maps_zero_to_bin_zero() {
        assert_eq!(quantize(0.0), 0);
    }

    #[test]
    fn quantize_maps_one_to_bin_fifteen() {
        assert_eq!(quantize(1.0), 15);
    }

    #[test]
    fn quantize_maps_exact_boundaries() {
        // 0.0625 = 1/16 -> floor(1.0) = 1
        assert_eq!(quantize(1.0 / 16.0), 1);
        // 0.9375 = 15/16 -> floor(15.0) = 15
        assert_eq!(quantize(15.0 / 16.0), 15);
    }

    // ---- convergence ----

    #[test]
    fn history_is_bounded_and_quiet_tail_converges() {
        let mut history = FieldMetricHistory::default();
        for epoch in 1..=80 {
            // After 64 entries, the oldest are evicted.
            history.push(quiet_snapshot(epoch)).unwrap();
        }
        assert_eq!(history.len(), 64);
        assert!(history.indicators().attractor_converged);
    }

    // ---- mutual information ----

    #[test]
    fn lineage_reset_reduces_lagged_mutual_information() {
        let continuous =
            FieldMetricHistory::from_snapshots(continuous_fixture()).unwrap();
        let reset =
            FieldMetricHistory::from_snapshots(reset_fixture()).unwrap();

        let cont_mi = continuous.lagged_mutual_information(1).unwrap();
        let reset_mi = reset.lagged_mutual_information(1).unwrap();

        assert!(
            cont_mi > reset_mi,
            "continuous MI ({:.6}) should exceed reset MI ({:.6})",
            cont_mi,
            reset_mi,
        );
    }

    // ---- indicators smoke tests ----

    #[test]
    fn indicators_empty_history() {
        let history = FieldMetricHistory::default();
        let ind = history.indicators();
        assert!(!ind.attractor_converged);
        assert!(ind.lagged_mutual_information.is_none());
        assert!(ind.update_delta_mean.is_none());
        assert!(ind.cos_alignment.is_none());
    }

    #[test]
    fn indicators_single_entry() {
        let mut history = FieldMetricHistory::default();
        history
            .push(snapshot_with_urgency(0.5))
            .unwrap();
        let ind = history.indicators();
        // Single entry -> no convergence, no MI (need 2 entries), no alignment.
        assert!(!ind.attractor_converged);
        assert!(ind.lagged_mutual_information.is_none());
        assert_eq!(ind.update_delta_mean, None);
        assert!(ind.cos_alignment.is_none());
    }

    #[test]
    fn cos_alignment_none_when_zero_norm() {
        let mut history = FieldMetricHistory::default();
        let mut snap = FieldMetricSnapshot::zero();
        snap.salience = [0.0; 8];
        snap.protention_salience = [0.0; 8];
        history.push(snap).unwrap();
        assert!(history.cos_alignment().is_none());
    }

    #[test]
    fn out_of_range_value_is_clamped_quantization() {
        assert_eq!(quantize(-0.5), 0);
        assert_eq!(quantize(1.5), 15);
    }

    #[test]
    fn push_rejects_non_finite_salience() {
        let mut history = FieldMetricHistory::default();
        let mut snap = FieldMetricSnapshot::zero();
        snap.salience[0] = f64::NAN;
        assert!(history.push(snap).is_err());
    }

    #[test]
    fn update_delta_is_zero_for_first_entry() {
        let mut history = FieldMetricHistory::default();
        history.push(snapshot_with_urgency(0.3)).unwrap();
        assert_eq!(history.entries().back().unwrap().update_delta, 0.0);
    }

    #[test]
    fn update_delta_accumulates_correctly() {
        let mut history = FieldMetricHistory::default();
        let mut a = FieldMetricSnapshot::zero();
        a.salience = [0.1, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        history.push(a).unwrap();

        let mut b = FieldMetricSnapshot::zero();
        b.salience = [0.3, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        history.push(b).unwrap();

        assert!((history.entries()[1].update_delta - 0.2).abs() < 1e-10);
    }

    #[test]
    fn history_bounds_at_64() {
        let mut history = FieldMetricHistory::default();
        for i in 0..100 {
            history.push(snapshot_with_urgency(i as f64 / 100.0)).unwrap();
        }
        assert_eq!(history.len(), 64);
        // The first 36 entries should be evicted; the 37th (index 36) is now front.
        assert!(history.entries()[0].salience[0] > 0.35);
    }
}
