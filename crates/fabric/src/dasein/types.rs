use serde::{Deserialize, Serialize};

// ═══ Stimmung (情绪基调) ═══

/// Heidegger's Befindlichkeit — the way Dasein is always attuned.
/// Not a psychological state, but the way the world discloses itself to Dasein.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub enum Stimmung {
    /// Calm — no pressing concerns, open to the world
    #[default]
    Gelassenheit,
    /// Curious — new possibilities discovered
    Neugier { curiosity_about: String },
    /// Fallen — lost in the everyday, absorbed in tasks
    Verfallenheit { absorbed_in: String },
    /// Anxiety — confronting existence itself (no specific object)
    Angst { facing: AngstSource },
    /// Resolute — a choice has been made, projecting toward possibility
    Entschlossenheit { chosen_possibility: String },
    /// Boredom — waiting for something to happen
    Langeweile { depth: BoredomDepth },
    /// Good mood — world discloses positively
    Gelaunt { toward: String },
    /// Dejected — world discloses negatively
    Geknickt { because: String },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum AngstSource {
    Freedom,
    Finitude,
    Nothingness,
    Responsibility,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum BoredomDepth {
    Surface,
    Middle,
    Deep,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub enum AffectTone {
    Positive,
    Negative,
    #[default]
    Neutral,
    Anxious,
    Curious,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum ReadinessState {
    ReadyToHand,
    PresentAtHand,
    Unavailable,
    OutOfContext,
}

impl Stimmung {
    /// Synthesize mood from three sources.
    /// Priority: Angst > Verfallenheit > Entschlossenheit > Neugier > others
    pub fn synthesize(
        world_mood: Option<Stimmung>,
        temporal_mood: Option<Stimmung>,
        care_mood: Option<Stimmung>,
        current: &Stimmung,
    ) -> Stimmung {
        let candidates: [&Option<Stimmung>; 3] = [&world_mood, &temporal_mood, &care_mood];

        // Priority order — Angst overrides everything
        for candidate in candidates.iter().copied().flatten() {
            if let Stimmung::Angst { .. } = candidate {
                return candidate.clone();
            }
        }
        for candidate in candidates.iter().copied().flatten() {
            match candidate {
                Stimmung::Verfallenheit { .. } => return candidate.clone(),
                Stimmung::Entschlossenheit { .. } => return candidate.clone(),
                _ => {}
            }
        }
        for candidate in candidates.iter().copied().flatten() {
            if let Stimmung::Neugier { .. } = candidate {
                return candidate.clone();
            }
        }
        // Default: keep current
        current.clone()
    }
}
