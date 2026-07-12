use fabric::dasein::{BoredomDepth, DaseinContext, DaseinEvent, Stimmung};
use fabric::self_field::MutationIntent;
use fabric::{wall_to_datetime, Clock};
use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;

pub struct MetaCognition {
    system_state: RwLock<SystemState>,
    decisions: RwLock<Vec<EvolutionDecision>>,
    thresholds: MetaCognitionThresholds,
    #[allow(dead_code)]
    dasein_tx: Option<mpsc::Sender<DaseinEvent>>,
    clock: Arc<dyn Clock>,
}

#[derive(Debug, Clone)]
pub struct SystemState {
    pub mood: Stimmung,
    pub turn_count: usize,
    pub last_evolution_turn: usize,
}

#[derive(Debug, Clone)]
pub struct EvolutionDecision {
    pub turn: usize,
    pub action: EvolutionAction,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone)]
pub enum EvolutionAction {
    Observe,
    TriggerEvolution { intents: Vec<MutationIntent> },
    AdjustDasein { parameter: String, value: f64 },
    InjectReflection { content: String },
}

#[derive(Debug, Clone)]
pub struct MetaCognitionThresholds {
    pub evolution_interval: usize,
}

impl Default for MetaCognitionThresholds {
    fn default() -> Self {
        Self {
            evolution_interval: 20,
        }
    }
}

impl MetaCognition {
    pub fn new(dasein_tx: Option<mpsc::Sender<DaseinEvent>>, clock: Arc<dyn Clock>) -> Self {
        Self {
            system_state: RwLock::new(SystemState {
                mood: Stimmung::Gelassenheit,
                turn_count: 0,
                last_evolution_turn: 0,
            }),
            decisions: RwLock::new(Vec::new()),
            thresholds: MetaCognitionThresholds::default(),
            dasein_tx,
            clock,
        }
    }

    pub fn decide(&self, ctx: &DaseinContext, turn: usize) -> EvolutionAction {
        let mut state = self.system_state.write().unwrap();
        state.turn_count = turn;
        state.mood = ctx.mood.clone();

        let action = match &ctx.mood {
            Stimmung::Angst { facing } => EvolutionAction::TriggerEvolution {
                intents: vec![MutationIntent {
                    target: "care.priorities".to_string(),
                    change: serde_json::json!({"action": "adjust", "magnitude": 0.1}),
                    reason: format!("Angst: {:?}", facing),
                    reversible: true,
                }],
            },
            Stimmung::Langeweile {
                depth: BoredomDepth::Deep,
            } => EvolutionAction::AdjustDasein {
                parameter: "curiosity_weight".to_string(),
                value: 0.8,
            },
            Stimmung::Neugier { curiosity_about } => EvolutionAction::InjectReflection {
                content: format!("Explore: {}", curiosity_about),
            },
            _ => {
                if turn - state.last_evolution_turn >= self.thresholds.evolution_interval {
                    state.last_evolution_turn = turn;
                    EvolutionAction::TriggerEvolution { intents: vec![] }
                } else {
                    EvolutionAction::Observe
                }
            }
        };

        self.decisions.write().unwrap().push(EvolutionDecision {
            turn,
            action: action.clone(),
            timestamp: wall_to_datetime(self.clock.wall_now()),
        });

        action
    }

    pub fn decisions(&self) -> Vec<EvolutionDecision> {
        self.decisions.read().unwrap().clone()
    }

    pub fn system_state(&self) -> SystemState {
        self.system_state.read().unwrap().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aletheon_kernel::chronos::TestClock;

    #[test]
    fn test_meta_cognition_observe() {
        let mc = MetaCognition::new(None, Arc::new(TestClock::default()));
        let state = mc.system_state();
        assert_eq!(state.turn_count, 0);
    }
}
