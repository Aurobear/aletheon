//! Temporality — Husserl's inner time consciousness.
//!
//! The temporal stream is the fundamental flow of experience:
//! retention (fading echo) -> primal impression (now) -> protention (expectation).
//! Not clock time, but lived time.

use super::types::*;
use base::dasein::{
    AffectTone, AngstSource, BoredomDepth, PresentSnapshot, ProtentionSnapshot, RentionalSnapshot,
    Stimmung, TemporalStreamSnapshot,
};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

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
                        content: format!("{} may repeat", what),
                        probability: 0.7,
                        consequence: "similar action as before".to_string(),
                        affect: AffectTone::Neutral,
                    });
                }
                TemporalPattern::Trend { direction, toward } => {
                    self.possibilities.push(AnticipatedPossibility {
                        content: format!("trend {} toward {}", direction, toward),
                        probability: 0.6,
                        consequence: "continuation of current trajectory".to_string(),
                        affect: AffectTone::Curious,
                    });
                }
                TemporalPattern::Disruption { what } => {
                    self.possibilities.push(AnticipatedPossibility {
                        content: format!("disruption in {}", what),
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
#[derive(Clone, Debug)]
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

/// Passive synthesizer — background meaning sedimentation.
/// Husserl: passive synthesis operates before active consciousness.
#[derive(Clone, Debug, Default)]
pub struct PassiveSynthesizer {
    pub associations: Vec<(String, String, f64)>, // (a, b, strength)
    pub habits: Vec<HabitEntry>,
    pub sediment_count: usize,
}

#[derive(Clone, Debug)]
pub struct HabitEntry {
    pub pattern: String,
    pub frequency: usize,
    pub last_seen: TemporalPosition,
}

impl PassiveSynthesizer {
    /// Run passive synthesis — called every N ticks.
    pub fn synthesize(&mut self, recent: &[RentionalMoment]) -> Vec<TemporalPattern> {
        self.sediment_count += 1;

        // Detect associations: if two concepts appear close together, link them
        for window in recent.windows(2) {
            let a = &window[0].content.semantic;
            let b = &window[1].content.semantic;

            // Check if association already exists
            if let Some(entry) = self
                .associations
                .iter_mut()
                .find(|(x, y, _)| (x == a && y == b) || (x == b && y == a))
            {
                entry.2 = (entry.2 + 0.1).min(1.0); // strengthen
            } else {
                self.associations.push((a.clone(), b.clone(), 0.1));
            }
        }

        // Detect habits: repeated patterns
        for moment in recent {
            if let Some(habit) = self
                .habits
                .iter_mut()
                .find(|h| h.pattern == moment.content.semantic)
            {
                habit.frequency += 1;
                habit.last_seen = moment.position;
            } else {
                self.habits.push(HabitEntry {
                    pattern: moment.content.semantic.clone(),
                    frequency: 1,
                    last_seen: moment.position,
                });
            }
        }

        // Prune weak associations
        self.associations
            .retain(|(_, _, strength)| *strength > 0.05);

        // Derive temporal patterns from accumulated state
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

        // Trend: strong associations (strength > 0.5) indicate a directional flow
        for (a, b, strength) in &self.associations {
            if *strength > 0.5 {
                patterns.push(TemporalPattern::Trend {
                    direction: format!("{} -> {}", a, b),
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
    pub fn ingest(&self, content: ExperientialContent, mood: Stimmung) {
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

    /// Run passive synthesis (called periodically).
    /// Returns detected temporal patterns.
    pub fn passive_synthesize(&self) -> Vec<TemporalPattern> {
        let vivid = self.retention.vivid_moments(0.3);
        let mut synth = self.synthesizer.write();
        synth.synthesize(&vivid)
    }

    /// Feed detected patterns into the protention field to close the prediction loop.
    pub fn update_protentions_from_patterns(&self, patterns: &[TemporalPattern]) {
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
            field.push_and_decay(make_moment(&format!("m{}", i), i));
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
            field.push_and_decay(make_moment(&format!("m{}", i), i));
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
}
