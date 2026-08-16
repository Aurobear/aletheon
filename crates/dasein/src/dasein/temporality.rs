//! Temporality — Husserl's inner time consciousness.
//!
//! The temporal stream is the fundamental flow of experience:
//! retention (fading echo) -> primal impression (now) -> protention (expectation).
//! Not clock time, but lived time.

use super::types::*;
use crate::dasein::context::{
    PresentSnapshot, ProtentionSnapshot, RentionalSnapshot, TemporalStreamSnapshot,
};
use ::contracts::dasein::{AffectTone, AngstSource, BoredomDepth, Stimmung};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};

// ═══ RetentionField (Task 2.1) ═══

/// A moment in the retention field — a fading echo of experience.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RentionalMoment {
    pub content: ExperientialContent,
    pub vividness: f64,
    pub significance: f64,
    pub affect: AffectTone,
    pub position: TemporalPosition,
    pub bewandtnis_links: Vec<EntityId>,
}

/// The content of an experience — not tokens, but lived experience.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExperientialContent {
    pub semantic: String,
    pub action: Option<String>,
    pub perception: Option<String>,
    pub negation: Option<String>,
}

/// Retention field — the fading echo of recent experience.
/// Husserl: retention is NOT memory. It's the still-living trace
/// of what just passed, fading like the tail of a comet.
pub struct RetentionField {
    moments: RwLock<VecDeque<RentionalMoment>>,
    depth: usize,
    /// Base decay rate. Modified by mood: anxious -> faster, calm -> slower.
    base_decay_rate: RwLock<f64>,
}

impl RetentionField {
    pub fn new(depth: usize, base_decay_rate: f64) -> Self {
        Self {
            moments: RwLock::new(VecDeque::with_capacity(depth)),
            depth,
            base_decay_rate: RwLock::new(base_decay_rate),
        }
    }

    /// Push a new moment and decay all existing ones.
    pub fn push_and_decay(&self, moment: RentionalMoment) {
        let mut moments = self.moments.write();

        // Decay existing moments
        for m in moments.iter_mut() {
            m.vividness *= self.effective_decay_rate();
            // Clamp to avoid floating point drift
            if m.vividness < 0.01 {
                m.vividness = 0.0;
            }
        }

        // Remove fully faded moments
        moments.retain(|m| m.vividness > 0.01);

        // Push new moment at front (most recent)
        moments.push_front(moment);

        // Enforce depth limit
        while moments.len() > self.depth {
            moments.pop_back();
        }
    }

    /// Get recent retentional moments for snapshot.
    pub fn recent_snapshots(&self, max: usize) -> Vec<RentionalSnapshot> {
        let moments = self.moments.read();
        moments
            .iter()
            .take(max)
            .map(|m| RentionalSnapshot {
                semantic: m.content.semantic.clone(),
                vividness: m.vividness,
                significance: m.significance,
                affect: m.affect.clone(),
                position: m.position.0,
            })
            .collect()
    }

    /// Find moments with high vividness (still "alive" in retention).
    pub fn vivid_moments(&self, threshold: f64) -> Vec<RentionalMoment> {
        let moments = self.moments.read();
        moments
            .iter()
            .filter(|m| m.vividness >= threshold)
            .cloned()
            .collect()
    }

    /// Total number of retained moments.
    pub fn len(&self) -> usize {
        self.moments.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.moments.read().is_empty()
    }

    /// Adjust decay rate based on mood.
    fn effective_decay_rate(&self) -> f64 {
        // Base rate is applied. Mood adjustment happens at the TemporalStream level.
        *self.base_decay_rate.read()
    }

    /// Update the base decay rate (called by TemporalStream when mood changes).
    pub fn set_decay_rate(&self, rate: f64) {
        *self.base_decay_rate.write() = rate.clamp(0.1, 1.0);
    }
}

// ═══ Urimpression and ProtentionField (Task 2.2) ═══

/// Urimpression — the living present, the "now" of experience.
/// Husserl: the primal impression is the absolute beginning,
/// the source from which all experience flows.
/// Its vividness is always 1.0 — the present cannot fade.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Urimpression {
    pub content: ExperientialContent,
    pub vividness: f64, // always 1.0
    pub thickness: std::time::Duration,
    pub mood_tone: Stimmung,
}

impl Urimpression {
    pub fn new(content: ExperientialContent, mood: Stimmung) -> Self {
        Self {
            content,
            vividness: 1.0,
            thickness: std::time::Duration::from_millis(100),
            mood_tone: mood,
        }
    }
}

impl Default for Urimpression {
    fn default() -> Self {
        Self {
            content: ExperientialContent {
                semantic: "silence".to_string(),
                action: None,
                perception: None,
                negation: None,
            },
            vividness: 1.0,
            thickness: std::time::Duration::from_millis(100),
            mood_tone: Stimmung::Gelassenheit,
        }
    }
}

/// Anticipated possibility in the protention field.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AnticipatedPossibility {
    pub content: String,
    pub probability: f64,
    pub consequence: String,
    pub affect: AffectTone,
}

/// Protention field — the horizon of expectation.
/// Husserl: protention is the directedness-toward what is coming.
/// Not a plan, but a pre-awareness, a readiness for what will come.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProtentionField {
    pub possibilities: Vec<AnticipatedPossibility>,
    pub certainty: f64,
}

impl ProtentionField {
    pub fn new() -> Self {
        Self {
            possibilities: Vec::new(),
            certainty: 0.0,
        }
    }

    /// Update protentions based on patterns detected in retention.
    pub fn update_from_patterns(&mut self, patterns: &[TemporalPattern]) {
        self.possibilities.clear();

        for pattern in patterns {
            match pattern {
                TemporalPattern::Repetition { what, interval: _ } => {
                    self.possibilities.push(AnticipatedPossibility {
                        content: format!("{what} may repeat"),
                        probability: 0.7,
                        consequence: "similar action as before".to_string(),
                        affect: AffectTone::Neutral,
                    });
                }
                TemporalPattern::Trend { direction, toward } => {
                    self.possibilities.push(AnticipatedPossibility {
                        content: format!("trend {direction} toward {toward}"),
                        probability: 0.6,
                        consequence: "continuation of current trajectory".to_string(),
                        affect: AffectTone::Curious,
                    });
                }
                TemporalPattern::Disruption { what } => {
                    self.possibilities.push(AnticipatedPossibility {
                        content: format!("disruption in {what}"),
                        probability: 0.5,
                        consequence: "unexpected change".to_string(),
                        affect: AffectTone::Anxious,
                    });
                }
            }
        }

        // Sort by probability descending
        self.possibilities.sort_by(|a, b| {
            b.probability
                .partial_cmp(&a.probability)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Update certainty as average probability
        if !self.possibilities.is_empty() {
            self.certainty = self
                .possibilities
                .iter()
                .map(|p| p.probability)
                .sum::<f64>()
                / self.possibilities.len() as f64;
        } else {
            self.certainty = 0.0;
        }
    }
}

impl Default for ProtentionField {
    fn default() -> Self {
        Self::new()
    }
}

/// Patterns detected in the temporal stream.
#[derive(Clone, Debug, PartialEq)]
pub enum TemporalPattern {
    Repetition { what: String, interval: u64 },
    Trend { direction: String, toward: String },
    Disruption { what: String },
}

// ═══ TemporalStream (Task 2.3) ═══

/// The unified temporal stream — retention + present + protention.
/// This IS Dasein's time-consciousness. Not a clock, but a lived flow.
pub struct TemporalStream {
    pub retention: RetentionField,
    pub present: RwLock<Urimpression>,
    pub protention: RwLock<ProtentionField>,
    pub tempo: RwLock<Tempo>,
    pub synthesizer: RwLock<PassiveSynthesizer>,
    /// Monotonically increasing position counter
    position: RwLock<TemporalPosition>,
}

/// Tempo — the rhythm of experience.
#[derive(Clone, Debug)]
pub struct Tempo {
    pub speed: f64,
    pub acceleration: f64,
}

impl Default for Tempo {
    fn default() -> Self {
        Self {
            speed: 1.0,
            acceleration: 0.0,
        }
    }
}

impl Tempo {
    /// Adjust tempo based on mood.
    pub fn set_from_mood(&mut self, mood: &Stimmung) {
        match mood {
            Stimmung::Angst { .. } => {
                self.speed = 2.0; // anxious: time feels faster
                self.acceleration = 0.5;
            }
            Stimmung::Langeweile { depth } => {
                self.speed = match depth {
                    BoredomDepth::Deep => 0.1,
                    BoredomDepth::Middle => 0.3,
                    BoredomDepth::Surface => 0.6,
                };
                self.acceleration = -0.1;
            }
            Stimmung::Entschlossenheit { .. } => {
                self.speed = 1.5;
                self.acceleration = 0.2;
            }
            Stimmung::Neugier { .. } => {
                self.speed = 1.3;
                self.acceleration = 0.1;
            }
            _ => {
                self.speed = 1.0;
                self.acceleration = 0.0;
            }
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct PassiveSynthesizer {
    pub associations: Vec<(String, String, f64)>, // (a, b, strength)
    /// Internal index: canonical sorted (min,max) pair -> position in associations Vec.
    association_index: HashMap<(String, String), usize>,
    pub habits: Vec<HabitEntry>,
    /// Internal index: pattern string -> position in habits Vec.
    habit_index: HashMap<String, usize>,
    pub sediment_count: usize,
    /// Exact cached pattern vector; rebuilt only on first call or when
    /// the pattern set/content/order changes.
    cached_patterns: Option<Vec<TemporalPattern>>,
    /// True when a public `synthesize` call rebuilt patterns that have not
    /// been applied to protention yet.  Set by public calls, cleared only
    /// after a successful internal materialization + protention update.
    projection_dirty: bool,
}

/// Return a canonical sorted pair key for bidirectional association lookup.
fn canonical_pair(a: &str, b: &str) -> (String, String) {
    if a <= b {
        (a.to_string(), b.to_string())
    } else {
        (b.to_string(), a.to_string())
    }
}

#[derive(Clone, Debug)]
pub struct HabitEntry {
    pub pattern: String,
    pub frequency: usize,
    pub last_seen: TemporalPosition,
}

impl PassiveSynthesizer {
    /// Run passive synthesis — called every N ticks.
    /// Public callers always receive a freshly materialized pattern vector.
    pub fn synthesize(&mut self, recent: &[RentionalMoment]) -> Vec<TemporalPattern> {
        // Public path: always force materialization.
        self.synthesize_impl(recent, true)
            .0
            .expect("force_materialize always produces Some")
    }

    /// Internal path: only materializes the full deterministic pattern vector
    /// on the first call, when the set/content/order of projected patterns
    /// changes, or when a prior public `synthesize` call has dirtied the
    /// projection cache.  On stable reflections the internal counts are
    /// updated exactly but no Vec allocation is performed.
    ///
    /// Returns `Some(patterns)` when a fresh projection was needed;
    /// `None` on stable reflections that did not alter the pattern set.
    pub(crate) fn synthesize_maybe_skip(
        &mut self,
        recent: &[RentionalMoment],
    ) -> Option<Vec<TemporalPattern>> {
        self.synthesize_impl(recent, false).0
    }

    /// Core implementation shared by public and internal paths.
    ///
    /// Updates all internal state (habits, associations, sediment) exactly as
    /// before.  When `force_materialize` is false, the full pattern vector is
    /// only rebuilt on threshold crossings, index changes, or when a prior
    /// public call has set [`projection_dirty`].
    ///
    /// Returns `(Some(patterns), true)` when a new pattern vector was
    /// materialized; `(None, false)` on stable reflections.
    fn synthesize_impl(
        &mut self,
        recent: &[RentionalMoment],
        force_materialize: bool,
    ) -> (Option<Vec<TemporalPattern>>, bool) {
        self.sediment_count += 1;
        let mut patterns_dirty = false;

        // Detect associations: use index for O(1) lookup per window.
        for window in recent.windows(2) {
            let a = &window[0].content.semantic;
            let b = &window[1].content.semantic;
            let key = canonical_pair(a, b);

            if let Some(&idx) = self.association_index.get(&key) {
                let old_strength = self.associations[idx].2;
                self.associations[idx].2 = (old_strength + 0.1).min(1.0);
                // Threshold crossing into pattern territory (> 0.5)
                if old_strength <= 0.5 && self.associations[idx].2 > 0.5 {
                    patterns_dirty = true;
                }
            } else {
                let idx = self.associations.len();
                self.associations.push((a.clone(), b.clone(), 0.1));
                self.association_index.insert(key, idx);
            }
        }

        // Detect habits: use index for O(1) lookup per moment.
        for moment in recent {
            if let Some(&idx) = self.habit_index.get(&moment.content.semantic) {
                let old_freq = self.habits[idx].frequency;
                self.habits[idx].frequency += 1;
                self.habits[idx].last_seen = moment.position;
                // Threshold crossing into pattern territory (>= 3)
                if old_freq < 3 && self.habits[idx].frequency >= 3 {
                    patterns_dirty = true;
                }
            } else {
                let idx = self.habits.len();
                self.habits.push(HabitEntry {
                    pattern: moment.content.semantic.clone(),
                    frequency: 1,
                    last_seen: moment.position,
                });
                self.habit_index
                    .insert(moment.content.semantic.clone(), idx);
            }
        }

        // Prune weak associations and rebuild index when elements are removed.
        let old_assoc_len = self.associations.len();
        self.associations
            .retain(|(_, _, strength)| *strength > 0.05);
        if self.associations.len() != old_assoc_len {
            self.association_index.clear();
            for (i, (a, b, _)) in self.associations.iter().enumerate() {
                self.association_index.insert(canonical_pair(a, b), i);
            }
            // Index shifting changes pattern order within associations section
            patterns_dirty = true;
        }

        // Determine whether the projection must be rebuilt this call.
        let call_dirty = force_materialize || patterns_dirty;
        // A prior public `synthesize` may have rebuilt the cache without
        // updating protention.  Honor that dirty flag only on internal path.
        let projection_stale = self.projection_dirty && !force_materialize;
        let rebuilt = call_dirty || projection_stale || self.cached_patterns.is_none();

        if rebuilt {
            let patterns = self.build_patterns();
            self.cached_patterns = Some(patterns.clone());
            if force_materialize {
                self.projection_dirty = true;
            }
            (Some(patterns), true)
        } else {
            (None, false)
        }
    }

    /// Build the deterministic pattern vector from current state.
    /// Order: habits (by insertion), then associations (by insertion).
    fn build_patterns(&self) -> Vec<TemporalPattern> {
        let mut patterns = Vec::new();

        // Repetition: habits with frequency >= 3
        for habit in &self.habits {
            if habit.frequency >= 3 {
                patterns.push(TemporalPattern::Repetition {
                    what: habit.pattern.clone(),
                    interval: habit.frequency as u64,
                });
            }
        }

        // Trend: strong associations (strength > 0.5)
        for (a, b, strength) in &self.associations {
            if *strength > 0.5 {
                patterns.push(TemporalPattern::Trend {
                    direction: format!("{a} -> {b}"),
                    toward: b.clone(),
                });
            }
        }

        patterns
    }
}

impl TemporalStream {
    pub fn new(retention_depth: usize, decay_rate: f64) -> Self {
        Self {
            retention: RetentionField::new(retention_depth, decay_rate),
            present: RwLock::new(Urimpression::default()),
            protention: RwLock::new(ProtentionField::new()),
            tempo: RwLock::new(Tempo::default()),
            synthesizer: RwLock::new(PassiveSynthesizer::default()),
            position: RwLock::new(TemporalPosition(0)),
        }
    }

    /// Ingest a new experience — the temporal flow advances.
    pub(crate) fn ingest(&self, content: ExperientialContent, mood: Stimmung) {
        let mut pos = self.position.write();
        let current = *pos;
        *pos = pos.next();
        drop(pos);

        // Current present becomes a retentional moment (skip the initial default)
        if current.0 > 0 {
            let old_present = {
                let present = self.present.read();
                RentionalMoment {
                    content: present.content.clone(),
                    vividness: 1.0, // just-retained, still vivid
                    significance: 0.5,
                    affect: AffectTone::Neutral,
                    position: current,
                    bewandtnis_links: vec![],
                }
            };

            // Push old present into retention (with decay)
            self.retention.push_and_decay(old_present);
        }

        // New content becomes the present
        {
            let mut present = self.present.write();
            *present = Urimpression::new(content, mood);
        }
    }

    /// Run passive synthesis and update protention when patterns change.
    ///
    /// On stable reflections where no habit/association crosses a projection
    /// threshold and no prior public call has changed the pattern set, the
    /// pattern vector is reused from cache, no Vec allocation is performed,
    /// and protention is not rebuilt.
    ///
    /// Returns `true` when protention was updated, `false` on stable
    /// reflections.
    pub(crate) fn passive_synthesize_and_update(&self) -> bool {
        let vivid = self.retention.vivid_moments(0.3);
        let mut synth = self.synthesizer.write();
        if let Some(patterns) = synth.synthesize_maybe_skip(&vivid) {
            synth.projection_dirty = false;
            drop(synth);
            self.protention.write().update_from_patterns(&patterns);
            true
        } else {
            false
        }
    }

    /// Apply patterns directly to the protention field.
    ///
    /// Always updates protention; callers that want to avoid redundant
    /// rebuilds should use [`passive_synthesize_and_update`] instead.
    #[allow(dead_code)]
    pub(crate) fn update_protentions_from_patterns(&self, patterns: &[TemporalPattern]) {
        self.protention.write().update_from_patterns(patterns);
    }

    /// Get current temporal position.
    pub fn current_position(&self) -> TemporalPosition {
        *self.position.read()
    }

    /// Generate snapshot for ABI transport.
    pub fn to_snapshot(&self) -> TemporalStreamSnapshot {
        let present = self.present.read();
        let protention = self.protention.read();
        let tempo = self.tempo.read();

        TemporalStreamSnapshot {
            recent_retentions: self.retention.recent_snapshots(5),
            present: PresentSnapshot {
                semantic: present.content.semantic.clone(),
                action: present.content.action.clone(),
                perception: present.content.perception.clone(),
                mood_tone: present.mood_tone.clone(),
            },
            protentions: protention
                .possibilities
                .iter()
                .map(|p| ProtentionSnapshot {
                    content: p.content.clone(),
                    probability: p.probability,
                    consequence: p.consequence.clone(),
                })
                .collect(),
            tempo: tempo.speed,
        }
    }

    /// Determine mood influence from temporal state.
    pub fn determine_mood(&self) -> Option<Stimmung> {
        let protention = self.protention.read();

        // If high certainty about negative outcome -> anxiety
        if protention.certainty > 0.7 {
            if let Some(first) = protention.possibilities.first() {
                if first.affect == AffectTone::Anxious {
                    return Some(Stimmung::Angst {
                        facing: AngstSource::Finitude,
                    });
                }
            }
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_moment(semantic: &str, position: u64) -> RentionalMoment {
        RentionalMoment {
            content: ExperientialContent {
                semantic: semantic.to_string(),
                action: None,
                perception: None,
                negation: None,
            },
            vividness: 1.0,
            significance: 0.5,
            affect: AffectTone::Neutral,
            position: TemporalPosition(position),
            bewandtnis_links: vec![],
        }
    }

    #[test]
    fn test_retention_push_and_decay() {
        let field = RetentionField::new(10, 0.8);

        field.push_and_decay(make_moment("first", 1));
        field.push_and_decay(make_moment("second", 2));
        field.push_and_decay(make_moment("third", 3));

        let moments = field.moments.read();
        assert_eq!(moments.len(), 3);

        // Most recent is first
        assert_eq!(moments[0].content.semantic, "third");
        assert_eq!(moments[0].vividness, 1.0); // just pushed

        // Older moments are decayed
        assert!(moments[1].vividness < 1.0); // "second" decayed once
        assert!(moments[2].vividness < moments[1].vividness); // "first" decayed twice
    }

    #[test]
    fn test_retention_depth_limit() {
        let field = RetentionField::new(3, 0.9);

        for i in 0..5 {
            field.push_and_decay(make_moment(&format!("m{i}"), i));
        }

        assert_eq!(field.len(), 3);
        // Only the 3 most recent survive
        let snapshots = field.recent_snapshots(10);
        assert_eq!(snapshots[0].semantic, "m4");
        assert_eq!(snapshots[1].semantic, "m3");
        assert_eq!(snapshots[2].semantic, "m2");
    }

    #[test]
    fn test_retention_fading() {
        let field = RetentionField::new(10, 0.5);

        field.push_and_decay(make_moment("first", 1));

        // Push many more to decay "first" below threshold
        for i in 2..20 {
            field.push_and_decay(make_moment(&format!("m{i}"), i));
        }

        // "first" should have faded out (vividness < 0.01)
        let vivid = field.vivid_moments(0.01);
        assert!(!vivid.iter().any(|m| m.content.semantic == "first"));
    }

    #[test]
    fn test_temporal_stream_ingest() {
        let stream = TemporalStream::new(10, 0.8);

        stream.ingest(
            ExperientialContent {
                semantic: "hello".to_string(),
                action: None,
                perception: None,
                negation: None,
            },
            Stimmung::Gelassenheit,
        );

        let present = stream.present.read();
        assert_eq!(present.content.semantic, "hello");
        assert_eq!(present.vividness, 1.0);

        // Second ingest moves "hello" to retention
        drop(present);
        stream.ingest(
            ExperientialContent {
                semantic: "world".to_string(),
                action: None,
                perception: None,
                negation: None,
            },
            Stimmung::Neugier {
                curiosity_about: "test".to_string(),
            },
        );

        let present = stream.present.read();
        assert_eq!(present.content.semantic, "world");

        let snapshots = stream.retention.recent_snapshots(5);
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].semantic, "hello");
    }

    #[test]
    fn test_temporal_stream_position() {
        let stream = TemporalStream::new(10, 0.8);

        assert_eq!(stream.current_position().0, 0);

        stream.ingest(
            ExperientialContent {
                semantic: "a".to_string(),
                action: None,
                perception: None,
                negation: None,
            },
            Stimmung::default(),
        );

        assert_eq!(stream.current_position().0, 1);
    }

    #[test]
    fn test_tempo_mood_adjustment() {
        let mut tempo = Tempo::default();

        tempo.set_from_mood(&Stimmung::Angst {
            facing: AngstSource::Freedom,
        });
        assert!(tempo.speed > 1.0);

        tempo.set_from_mood(&Stimmung::Langeweile {
            depth: BoredomDepth::Deep,
        });
        assert!(tempo.speed < 0.5);
    }

    #[test]
    fn test_passive_synthesis() {
        let mut synth = PassiveSynthesizer::default();

        let moments = vec![
            RentionalMoment {
                content: ExperientialContent {
                    semantic: "code".to_string(),
                    action: None,
                    perception: None,
                    negation: None,
                },
                vividness: 0.8,
                significance: 0.5,
                affect: AffectTone::Neutral,
                position: TemporalPosition(1),
                bewandtnis_links: vec![],
            },
            RentionalMoment {
                content: ExperientialContent {
                    semantic: "test".to_string(),
                    action: None,
                    perception: None,
                    negation: None,
                },
                vividness: 0.7,
                significance: 0.5,
                affect: AffectTone::Neutral,
                position: TemporalPosition(2),
                bewandtnis_links: vec![],
            },
        ];

        let _patterns = synth.synthesize(&moments);

        assert_eq!(synth.associations.len(), 1);
        assert_eq!(synth.associations[0].0, "code");
        assert_eq!(synth.associations[0].1, "test");
        assert_eq!(synth.habits.len(), 2);
    }

    #[test]
    fn first_pattern_projection_is_not_skipped() {
        let stream = TemporalStream::new(8, 0.95);
        let patterns = vec![TemporalPattern::Repetition {
            what: "code".to_string(),
            interval: 3,
        }];

        stream.update_protentions_from_patterns(&patterns);

        let protention = stream.protention.read();
        assert_eq!(protention.possibilities.len(), 1);
        assert_eq!(protention.possibilities[0].content, "code may repeat");
        assert_eq!(protention.certainty, 0.7);
    }

    /// Regression: after ingesting lived events and running the combined
    /// synthesis+protention path, the protention field must contain
    /// non-empty expectations with meaningful certainty.
    #[test]
    fn lived_events_produce_non_empty_protentions() {
        let stream = TemporalStream::new(20, 0.95);

        // Ingest overlapping lived events so habits and associations cross
        // their projection thresholds.
        let events = [
            ("code", "compile"),
            ("test", "verify"),
            ("code", "compile"),
            ("test", "verify"),
            ("code", "compile"), // 3rd "code" -> repetition pattern
            ("test", "verify"),
            ("code", "compile"),
            ("test", "verify"),
            ("code", "compile"),
            ("test", "verify"),
            ("code", "compile"), // 6th pair: association > 0.5 -> trend
        ];

        for (semantic, action) in &events {
            stream.ingest(
                ExperientialContent {
                    semantic: semantic.to_string(),
                    action: Some(action.to_string()),
                    perception: None,
                    negation: None,
                },
                Stimmung::Gelassenheit,
            );
        }

        // Run passive synthesis through the combined path.
        let changed = stream.passive_synthesize_and_update();
        assert!(
            changed,
            "synthesis must detect changes after ingesting repeated events"
        );

        // Verify protention field was populated.
        let protention = stream.protention.read();
        assert!(
            !protention.possibilities.is_empty(),
            "protention field must contain expected possibilities"
        );
        assert!(
            protention.certainty > 0.0,
            "protention certainty must be non-zero after pattern-driven update"
        );

        // At least one repetition for "code" (appeared >= 3 times).
        // Re-read the synthesizer to inspect cached patterns.
        let synth = stream.synthesizer.read();
        let cached = synth
            .cached_patterns
            .as_ref()
            .expect("cached patterns must be populated");
        let has_code_repetition = cached
            .iter()
            .any(|p| matches!(p, TemporalPattern::Repetition { what, .. } if what == "code"));
        assert!(
            has_code_repetition,
            "'code' must generate a repetition pattern"
        );
    }

    /// The optimized `synthesize_maybe_skip` path must return `None` on
    /// every stable call once the projection has been materialized, proving
    /// no Vec allocation is performed on the hot path.  1,000 stable
    /// reflections must each return `None`.
    #[test]
    fn optimized_path_bounds_pattern_materializations() {
        let mut synth = PassiveSynthesizer::default();

        // Seed a habit that crosses the repetition threshold.
        let moments: Vec<_> = (0..3)
            .map(|i| RentionalMoment {
                content: ExperientialContent {
                    semantic: "code".to_string(),
                    action: None,
                    perception: None,
                    negation: None,
                },
                vividness: 0.8,
                significance: 0.5,
                affect: AffectTone::Neutral,
                position: TemporalPosition(i),
                bewandtnis_links: vec![],
            })
            .collect();

        // First call: must produce Some (cached is None).
        let patterns = synth.synthesize_maybe_skip(&moments);
        assert!(patterns.is_some(), "first call must rebuild patterns");

        // Subsequent calls with stable state must return None (no allocation).
        let mut rebuild_count = 1usize;

        for i in 0..1_000 {
            let moment = vec![RentionalMoment {
                content: ExperientialContent {
                    semantic: format!("noise_{i}"),
                    action: None,
                    perception: None,
                    negation: None,
                },
                vividness: 0.8,
                significance: 0.5,
                affect: AffectTone::Neutral,
                position: TemporalPosition(100 + i),
                bewandtnis_links: vec![],
            }];
            if synth.synthesize_maybe_skip(&moment).is_some() {
                rebuild_count += 1;
            }
        }

        assert_eq!(
            rebuild_count, 1,
            "only the first call should rebuild; threshold already crossed"
        );
    }

    /// After a public `synthesize` call crosses a threshold that was
    /// previously unmet, the next internal call must rebuild the projection
    /// and populate protention.  Once applied, dirty is cleared.
    #[test]
    fn threshold_crossing_after_public_synthesize_produces_some_and_populates_protention() {
        let stream = TemporalStream::new(20, 0.95);

        // Ingest "task" twice (below repetition threshold of 3).
        for _ in 0..2 {
            stream.ingest(
                ExperientialContent {
                    semantic: "task".to_string(),
                    action: Some("review".into()),
                    perception: None,
                    negation: None,
                },
                Stimmung::Gelassenheit,
            );
        }

        // Flush retention so the initial "task" moments are evicted.
        for i in 0..25 {
            stream.ingest(
                ExperientialContent {
                    semantic: format!("flush_{i}"),
                    action: None,
                    perception: None,
                    negation: None,
                },
                Stimmung::Gelassenheit,
            );
        }

        // First synthesis initialises the cache.
        let _ = stream.passive_synthesize_and_update();

        // Public synthesize: feed 3 "task" moments, crossing the
        // repetition threshold.  This sets projection_dirty.
        let cross_moments: Vec<_> = (0..3)
            .map(|i| RentionalMoment {
                content: ExperientialContent {
                    semantic: "task".to_string(),
                    action: Some("review".into()),
                    perception: None,
                    negation: None,
                },
                vividness: 0.8,
                significance: 0.5,
                affect: AffectTone::Neutral,
                position: TemporalPosition(500 + i),
                bewandtnis_links: vec![],
            })
            .collect();
        {
            let mut synth = stream.synthesizer.write();
            let public_patterns = synth.synthesize(&cross_moments);
            assert!(
                !public_patterns.is_empty(),
                "public call must see repetition pattern"
            );
            assert!(
                synth.projection_dirty,
                "public call must set projection_dirty"
            );
        }

        // Advance retention so the next synthesis processes fresh data
        // (flush events are shifted forward, avoiding duplicate counting).
        for i in 0..22 {
            stream.ingest(
                ExperientialContent {
                    semantic: format!("advance_{i}"),
                    action: None,
                    perception: None,
                    negation: None,
                },
                Stimmung::Gelassenheit,
            );
        }

        // Internal path must rebuild (projection_dirty is true) and
        // populate protention.
        let changed = stream.passive_synthesize_and_update();
        assert!(changed, "internal path after public call must rebuild");

        let protention = stream.protention.read();
        assert!(
            !protention.possibilities.is_empty(),
            "protention must be populated after rebuild"
        );
        drop(protention);

        // Verify dirty was cleared.
        assert!(
            !stream.synthesizer.read().projection_dirty,
            "projection_dirty must be cleared after protention update"
        );
    }
}
