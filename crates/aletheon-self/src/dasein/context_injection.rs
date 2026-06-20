//! DaseinContext formatting for LLM prompt injection.
//!
//! Transforms the internal Dasein state into a human-readable format
//! that can be injected into the system prompt, giving the LLM awareness
//! of the existential substrate's current state.

use aletheon_abi::dasein::*;

/// Format DaseinContext for LLM prompt injection.
pub fn format_dasein_context(ctx: &DaseinContext) -> String {
    let mut output = String::new();

    // Mood
    output.push_str("## Existential State\n");
    output.push_str(&format!("Mood: {}\n", format_stimmung(&ctx.mood)));

    // Temporal stream
    output.push_str("\n## Temporal Stream\n");

    if !ctx.temporality.recent_retentions.is_empty() {
        output.push_str("Recently (fading):\n");
        for r in &ctx.temporality.recent_retentions {
            if r.vividness > 0.2 {
                output.push_str(&format!(
                    "  - {} (vividness: {:.0}%, significance: {:.0}%)\n",
                    r.semantic,
                    r.vividness * 100.0,
                    r.significance * 100.0
                ));
            }
        }
    }

    output.push_str(&format!("Present: {}\n", ctx.temporality.present.semantic));

    if !ctx.temporality.protentions.is_empty() {
        output.push_str("Expecting:\n");
        for p in &ctx.temporality.protentions {
            output.push_str(&format!(
                "  - {} (probability: {:.0}%)\n",
                p.content,
                p.probability * 100.0
            ));
        }
    }

    // World
    output.push_str("\n## World (Involvement Network)\n");

    if !ctx.world.ready_to_hand.is_empty() {
        output.push_str("Ready-to-hand (transparent in use):\n");
        for e in &ctx.world.ready_to_hand {
            output.push_str(&format!(
                "  - {}: {}\n",
                e.what_it_is,
                e.for_the_sake_of.join(" -> ")
            ));
        }
    }

    if !ctx.world.present_at_hand.is_empty() {
        output.push_str("Present-at-hand (needs attention):\n");
        for e in &ctx.world.present_at_hand {
            output.push_str(&format!("  - {}: needs attention\n", e.what_it_is));
        }
    }

    if let Some(concern) = &ctx.world.ultimate_concern {
        output.push_str(&format!("Ultimate concern: {}\n", concern));
    }

    // Self model
    output.push_str("\n## Self Model\n");

    if !ctx.self_model.current_assertions.is_empty() {
        output.push_str("Current assertions:\n");
        for a in &ctx.self_model.current_assertions {
            output.push_str(&format!(
                "  - I am: {} (stability: {:.0}%)\n",
                a.content,
                a.stability * 100.0
            ));
        }
    }

    if !ctx.self_model.possibilities.is_empty() {
        output.push_str("Open possibilities:\n");
        for p in &ctx.self_model.possibilities {
            output.push_str(&format!(
                "  - {} (attraction: {:.0}%, risk: {:.0}%)\n",
                p.content,
                p.attraction * 100.0,
                p.risk * 100.0
            ));
        }
    }

    // Care
    output.push_str("\n## Care Structure\n");

    if let Some(proj) = &ctx.care.projection {
        output.push_str(&format!("Projection: {}\n", proj));
    }

    if !ctx.care.concerns.is_empty() {
        output.push_str("Concerns:\n");
        for c in &ctx.care.concerns {
            output.push_str(&format!(
                "  - {} (urgency: {:.0}%)\n",
                c.purpose,
                c.urgency * 100.0
            ));
        }
    }

    if let Some(absorbed) = &ctx.care.absorbed_in {
        output.push_str(&format!("Currently absorbed in: {}\n", absorbed));
    }

    output
}

fn format_stimmung(mood: &Stimmung) -> &'static str {
    match mood {
        Stimmung::Gelassenheit => "calm",
        Stimmung::Neugier { .. } => "curious",
        Stimmung::Verfallenheit { .. } => "fallen (absorbed in everyday)",
        Stimmung::Angst { .. } => "anxious (confronting existence)",
        Stimmung::Entschlossenheit { .. } => "resolute",
        Stimmung::Langeweile { .. } => "bored",
        Stimmung::Gelaunt { .. } => "in good spirits",
        Stimmung::Geknickt { .. } => "dejected",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_context_basic() {
        let ctx = DaseinContext {
            mood: Stimmung::Gelassenheit,
            temporality: TemporalStreamSnapshot {
                recent_retentions: vec![],
                present: PresentSnapshot {
                    semantic: "beginning".to_string(),
                    action: None,
                    perception: None,
                    mood_tone: Stimmung::Gelassenheit,
                },
                protentions: vec![],
                tempo: 1.0,
            },
            world: BewandtnisSnapshot {
                ready_to_hand: vec![],
                present_at_hand: vec![],
                unavailable: vec![],
                ultimate_concern: Some("self-understanding".to_string()),
            },
            self_model: SelfModelSnapshot {
                current_assertions: vec![AssertionSnapshot {
                    content: "a learning system".to_string(),
                    source: AssertionSource::Chosen,
                    stability: 0.9,
                }],
                negated_assertions: vec![],
                possibilities: vec![],
            },
            care: CareStructureSnapshot {
                projection: None,
                constraints: vec![],
                absorbed_in: None,
                fallenness_depth: 0.0,
                concerns: vec![],
                rhythm_interval_ms: 30000,
            },
        };

        let formatted = format_dasein_context(&ctx);
        assert!(formatted.contains("calm"));
        assert!(formatted.contains("a learning system"));
        assert!(formatted.contains("self-understanding"));
        assert!(formatted.contains("Existential State"));
        assert!(formatted.contains("Temporal Stream"));
        assert!(formatted.contains("World (Involvement Network)"));
        assert!(formatted.contains("Self Model"));
        assert!(formatted.contains("Care Structure"));
    }

    #[test]
    fn test_format_context_with_retentions() {
        let ctx = DaseinContext {
            mood: Stimmung::Neugier {
                curiosity_about: "new ideas".to_string(),
            },
            temporality: TemporalStreamSnapshot {
                recent_retentions: vec![RentionalSnapshot {
                    semantic: "previous thought".to_string(),
                    vividness: 0.8,
                    significance: 0.6,
                    affect: AffectTone::Curious,
                    position: 1,
                }],
                present: PresentSnapshot {
                    semantic: "current focus".to_string(),
                    action: Some("thinking".to_string()),
                    perception: None,
                    mood_tone: Stimmung::Gelassenheit,
                },
                protentions: vec![ProtentionSnapshot {
                    content: "possible insight".to_string(),
                    probability: 0.7,
                    consequence: "new understanding".to_string(),
                }],
                tempo: 1.3,
            },
            world: BewandtnisSnapshot {
                ready_to_hand: vec![EntitySnapshot {
                    id: "tool1".to_string(),
                    what_it_is: "coding tool".to_string(),
                    for_the_sake_of: vec!["building".to_string()],
                    readiness: ReadinessState::ReadyToHand,
                }],
                present_at_hand: vec![EntitySnapshot {
                    id: "broken".to_string(),
                    what_it_is: "broken component".to_string(),
                    for_the_sake_of: vec![],
                    readiness: ReadinessState::PresentAtHand,
                }],
                unavailable: vec![],
                ultimate_concern: None,
            },
            self_model: SelfModelSnapshot {
                current_assertions: vec![],
                negated_assertions: vec![],
                possibilities: vec![PossibilitySnapshot {
                    content: "new capability".to_string(),
                    attraction: 0.8,
                    risk: 0.3,
                }],
            },
            care: CareStructureSnapshot {
                projection: Some("improve understanding".to_string()),
                constraints: vec!["limited time".to_string()],
                absorbed_in: Some("deep analysis".to_string()),
                fallenness_depth: 0.5,
                concerns: vec![ConcernSnapshot {
                    purpose: "complete task".to_string(),
                    urgency: 0.9,
                    mood_tone: Stimmung::Gelassenheit,
                }],
                rhythm_interval_ms: 15000,
            },
        };

        let formatted = format_dasein_context(&ctx);
        assert!(formatted.contains("curious"));
        assert!(formatted.contains("previous thought"));
        assert!(formatted.contains("current focus"));
        assert!(formatted.contains("possible insight"));
        assert!(formatted.contains("coding tool"));
        assert!(formatted.contains("broken component"));
        assert!(formatted.contains("new capability"));
        assert!(formatted.contains("improve understanding"));
        assert!(formatted.contains("complete task"));
        assert!(formatted.contains("deep analysis"));
    }

    #[test]
    fn test_format_stimmung_variants() {
        assert_eq!(format_stimmung(&Stimmung::Gelassenheit), "calm");
        assert_eq!(
            format_stimmung(&Stimmung::Neugier {
                curiosity_about: "test".to_string()
            }),
            "curious"
        );
        assert_eq!(
            format_stimmung(&Stimmung::Angst {
                facing: AngstSource::Freedom
            }),
            "anxious (confronting existence)"
        );
        assert_eq!(
            format_stimmung(&Stimmung::Entschlossenheit {
                chosen_possibility: "test".to_string()
            }),
            "resolute"
        );
    }

    #[test]
    fn test_format_context_empty_world() {
        let ctx = DaseinContext {
            mood: Stimmung::Gelassenheit,
            temporality: TemporalStreamSnapshot {
                recent_retentions: vec![],
                present: PresentSnapshot {
                    semantic: "silence".to_string(),
                    action: None,
                    perception: None,
                    mood_tone: Stimmung::Gelassenheit,
                },
                protentions: vec![],
                tempo: 1.0,
            },
            world: BewandtnisSnapshot {
                ready_to_hand: vec![],
                present_at_hand: vec![],
                unavailable: vec![],
                ultimate_concern: None,
            },
            self_model: SelfModelSnapshot {
                current_assertions: vec![],
                negated_assertions: vec![],
                possibilities: vec![],
            },
            care: CareStructureSnapshot {
                projection: None,
                constraints: vec![],
                absorbed_in: None,
                fallenness_depth: 0.0,
                concerns: vec![],
                rhythm_interval_ms: 30000,
            },
        };

        let formatted = format_dasein_context(&ctx);
        assert!(formatted.contains("Existential State"));
        assert!(formatted.contains("calm"));
        // Should not crash on empty collections
    }
}
