use super::types::*;
use fabric::dasein::{CareStructureSnapshot, ConcernSnapshot, Stimmung};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A concern — something Dasein cares about.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Concern {
    pub id: String,
    pub purpose: String,
    pub urgency: f64,
    pub involvement_chain: Vec<Involvement>,
    pub last_attended: TemporalPosition,
    pub mood_tone: Stimmung,
}

/// Projection — Dasein's being-ahead-of-itself.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Projection {
    pub possibilities: Vec<ProjectedPossibility>,
    pub chosen: Option<String>,
    pub for_the_sake_of: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProjectedPossibility {
    pub description: String,
    pub source: PossibilitySource,
    pub attraction: f64,
    pub risk: f64,
    pub conditions: Vec<EntityId>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum PossibilitySource {
    FromNegation,
    FromWorld,
    FromProjection,
    FromUser,
}

/// Thrownness — Dasein's already-being-in-a-world.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Thrownness {
    pub constraints: Vec<Constraint>,
    pub initial_conditions: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Constraint {
    pub description: String,
    pub challengeable: bool,
}

/// Fallenness — Dasein's being-alongside the world.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Fallenness {
    pub absorbed_in: Option<String>,
    pub depth: f64,
    pub wake_triggers: Vec<String>,
}

/// The care rhythm — how often the sorge loop ticks.
#[derive(Clone, Debug)]
pub struct CareRhythm {
    pub base_interval: std::time::Duration,
    pub current_interval: std::time::Duration,
    pub min_interval: std::time::Duration,
    pub max_interval: std::time::Duration,
}

impl Default for CareRhythm {
    fn default() -> Self {
        Self {
            base_interval: std::time::Duration::from_secs(30),
            current_interval: std::time::Duration::from_secs(30),
            min_interval: std::time::Duration::from_secs(5),
            max_interval: std::time::Duration::from_secs(300),
        }
    }
}

impl CareRhythm {
    /// Adapt rhythm based on mood and concerns.
    pub fn adapt(&mut self, mood: &Stimmung, urgent_count: usize) {
        let mood_factor = match mood {
            Stimmung::Angst { .. } => 0.3,            // anxious: check more often
            Stimmung::Entschlossenheit { .. } => 0.5, // resolute: fairly often
            Stimmung::Neugier { .. } => 0.7,          // curious: moderately often
            Stimmung::Gelassenheit => 1.0,            // calm: normal pace
            Stimmung::Langeweile { .. } => 2.0,       // bored: less often
            Stimmung::Verfallenheit { .. } => 1.5,    // fallen: less aware
            _ => 1.0,
        };

        let urgency_factor = if urgent_count > 0 {
            1.0 / (1.0 + urgent_count as f64)
        } else {
            1.0
        };

        let new_interval = self
            .base_interval
            .mul_f64(mood_factor)
            .mul_f64(urgency_factor);

        self.current_interval = new_interval.max(self.min_interval).min(self.max_interval);
    }

    pub fn next_interval(&self) -> std::time::Duration {
        self.current_interval
    }
}

/// CareAction — what the sorge loop decides to do.
#[derive(Clone, Debug)]
pub enum CareAction {
    /// Deep deliberation needed — spawn ReAct loop
    Deliberate(String),
    /// Direct action — no deliberation needed
    Direct(String),
    /// Wait — monitoring but not acting
    Wait(String),
    /// Negate — question something about self
    Negate(String),
}

/// The care structure — Dasein's unified mode of being.
/// Heidegger: Sorge (care) = projection + thrownness + fallenness.
pub struct CareStructure {
    projection: RwLock<Projection>,
    thrownness: RwLock<Thrownness>,
    fallenness: RwLock<Fallenness>,
    concerns: RwLock<BTreeMap<String, Concern>>,
    pub(crate) rhythm: RwLock<CareRhythm>,
}

impl CareStructure {
    pub fn new() -> Self {
        Self {
            projection: RwLock::new(Projection {
                possibilities: Vec::new(),
                chosen: None,
                for_the_sake_of: "self-understanding".to_string(),
            }),
            thrownness: RwLock::new(Thrownness {
                constraints: Vec::new(),
                initial_conditions: Vec::new(),
            }),
            fallenness: RwLock::new(Fallenness {
                absorbed_in: None,
                depth: 0.0,
                wake_triggers: Vec::new(),
            }),
            concerns: RwLock::new(BTreeMap::new()),
            rhythm: RwLock::new(CareRhythm::default()),
        }
    }

    /// Add or update a concern.
    pub(crate) fn add_concern(&self, concern: Concern) {
        let mut concerns = self.concerns.write();
        concerns.insert(concern.id.clone(), concern);
    }

    /// Get urgent concerns.
    pub fn urgent_concerns(&self, threshold: f64) -> Vec<Concern> {
        let concerns = self.concerns.read();
        concerns
            .values()
            .filter(|c| c.urgency >= threshold)
            .cloned()
            .collect()
    }

    /// Determine what action to take.
    pub fn determine_action(&self) -> CareAction {
        let concerns = self.concerns.read();
        let fallenness = self.fallenness.read();
        let projection = self.projection.read();

        // If absorbed in something and depth is high -> wake up
        if fallenness.depth > 0.8 {
            if let Some(absorbed) = &fallenness.absorbed_in {
                return CareAction::Negate(format!(
                    "deeply absorbed in '{absorbed}', questioning this pattern"
                ));
            }
        }

        // If there are urgent concerns -> deliberate
        let urgent: Vec<_> = concerns.values().filter(|c| c.urgency > 0.7).collect();

        if let Some(most_urgent) = urgent.first() {
            return CareAction::Deliberate(most_urgent.purpose.clone());
        }

        // If there's a chosen projection -> act on it
        if let Some(chosen) = &projection.chosen {
            return CareAction::Direct(chosen.clone());
        }

        // Default: wait
        CareAction::Wait("no urgent concerns, monitoring".to_string())
    }

    /// Update fallenness state.
    #[cfg(test)]
    pub(crate) fn update_fallenness(&self, absorbed: Option<String>, depth: f64) {
        let mut fallenness = self.fallenness.write();
        fallenness.absorbed_in = absorbed;
        fallenness.depth = depth;
    }

    /// Determine mood from care state.
    pub fn determine_mood(&self) -> Option<Stimmung> {
        let fallenness = self.fallenness.read();
        let concerns = self.concerns.read();

        // Deep fallenness -> Verfallenheit
        if fallenness.depth > 0.7 {
            if let Some(absorbed) = &fallenness.absorbed_in {
                return Some(Stimmung::Verfallenheit {
                    absorbed_in: absorbed.clone(),
                });
            }
        }

        // Multiple urgent concerns -> Angst
        let urgent_count = concerns.values().filter(|c| c.urgency > 0.7).count();

        if urgent_count >= 3 {
            return Some(Stimmung::Angst {
                facing: fabric::dasein::AngstSource::Responsibility,
            });
        }

        None
    }

    /// Generate snapshot.
    pub fn to_snapshot(&self) -> CareStructureSnapshot {
        let proj = self.projection.read();
        let thrown = self.thrownness.read();
        let fallen = self.fallenness.read();
        let concerns = self.concerns.read();
        let rhythm = self.rhythm.read();

        CareStructureSnapshot {
            projection: proj.chosen.clone(),
            constraints: thrown
                .constraints
                .iter()
                .map(|c| c.description.clone())
                .collect(),
            absorbed_in: fallen.absorbed_in.clone(),
            fallenness_depth: fallen.depth,
            concerns: concerns
                .values()
                .map(|c| ConcernSnapshot {
                    purpose: c.purpose.clone(),
                    urgency: c.urgency,
                    mood_tone: c.mood_tone.clone(),
                })
                .collect(),
            rhythm_interval_ms: rhythm.current_interval.as_millis() as u64,
        }
    }
}

impl Default for CareStructure {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_concern(id: &str, urgency: f64) -> Concern {
        Concern {
            id: id.to_string(),
            purpose: format!("purpose of {id}"),
            urgency,
            involvement_chain: vec![],
            last_attended: TemporalPosition(0),
            mood_tone: Stimmung::Gelassenheit,
        }
    }

    #[test]
    fn test_concerns() {
        let care = CareStructure::new();

        care.add_concern(make_concern("a", 0.3));
        care.add_concern(make_concern("b", 0.8));
        care.add_concern(make_concern("c", 0.9));

        let urgent = care.urgent_concerns(0.7);
        assert_eq!(urgent.len(), 2);
    }

    #[test]
    fn test_determine_action_deliberate() {
        let care = CareStructure::new();
        care.add_concern(make_concern("urgent", 0.9));

        let action = care.determine_action();
        assert!(matches!(action, CareAction::Deliberate(_)));
    }

    #[test]
    fn test_determine_action_negate_fallenness() {
        let care = CareStructure::new();
        care.update_fallenness(Some("endless debugging".to_string()), 0.9);

        let action = care.determine_action();
        assert!(matches!(action, CareAction::Negate(_)));
    }

    #[test]
    fn test_rhythm_adaptation() {
        let care = CareStructure::new();

        // Calm mood -> normal rhythm
        care.rhythm.write().adapt(&Stimmung::Gelassenheit, 0);
        let calm_interval = care.rhythm.read().current_interval;

        // Angst -> faster rhythm
        care.rhythm.write().adapt(
            &Stimmung::Angst {
                facing: fabric::dasein::AngstSource::Freedom,
            },
            2,
        );
        let angst_interval = care.rhythm.read().current_interval;

        assert!(angst_interval < calm_interval);
    }
}
