//! Rendering for the sanitized conscious-core inspector contract.

use fabric::{ConsciousCoreSnapshot, ProcessorHealth};

pub fn render_snapshot(snapshot: &ConsciousCoreSnapshot) -> Vec<String> {
    let mut lines = vec![format!(
        "Conscious core — epoch {} — Dasein v{}",
        snapshot.epoch.0, snapshot.dasein_version.0
    )];
    for disposition in &snapshot.dispositions {
        let role = if disposition.winner {
            "winner"
        } else {
            "coalition"
        };
        lines.push(format!(
            "{role}: {} [{}; {}; {}] salience={:.2}",
            disposition.id.0,
            disposition.source_kind,
            disposition.content_schema,
            disposition.visibility,
            disposition.salience.goal_relevance
        ));
    }
    for ack in &snapshot.acknowledgements {
        let degraded = if ack.health != ProcessorHealth::Healthy {
            format!(
                " — degraded: {}",
                ack.degraded_reason.as_deref().unwrap_or("unknown")
            )
        } else {
            String::new()
        };
        lines.push(format!(
            "processor {}: {:?} (accepted {}, rejected {}){degraded}",
            ack.processor.0, ack.health, ack.accepted_count, ack.rejected_count
        ));
    }
    lines.extend(
        snapshot
            .indicator_limitations
            .iter()
            .map(|value| format!("limitation: {value}")),
    );
    lines
}
