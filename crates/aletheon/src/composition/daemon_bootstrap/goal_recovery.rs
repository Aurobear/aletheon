//! Goal-state recovery performed before daemon request handling starts.

use adapters_sqlite::goal::ObjectiveStore;
use anyhow::Context;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{info, warn};

pub(super) async fn recover(
    objective_store: &Arc<Mutex<ObjectiveStore>>,
) -> anyhow::Result<Option<(String, Vec<String>)>> {
    // Terminalize stale runtime calls before making their Goals ready. Recovery
    // records cancellation evidence and never invokes a runtime.
    {
        let store = objective_store.lock().await;
        let stale_attempts = store
            .recover_stale_attempts()
            .context("recovering stale goal attempts")?;
        if !stale_attempts.is_empty() {
            info!(
                count = stale_attempts.len(),
                "Cancelled stale goal attempts on start"
            );
        }

        match store.recover_goals() {
            Ok(recovered) if !recovered.is_empty() => {
                info!(
                    count = recovered.len(),
                    "Recovered persisted goals on start"
                );
                for goal in &recovered {
                    info!(
                        goal_id = goal.id.0,
                        state = %goal.state,
                        version = goal.version,
                        "Goal recovered"
                    );
                }
            }
            Ok(_) => info!("No goals to recover"),
            Err(error) => warn!(error = %error, "Failed to recover goals on start"),
        }
    }

    // Resume the active objective only as a continuity projection. The goal
    // repository remains the authoritative state owner.
    let store = objective_store.lock().await;
    Ok(match store.resume() {
        Ok(Some((objective, sub_goals))) => {
            let descriptions = sub_goals
                .iter()
                .map(|sub_goal| sub_goal.description.clone())
                .collect::<Vec<_>>();
            info!(
                objective_id = objective.objective_id,
                description = %objective.description,
                sub_goals = descriptions.len(),
                "Resuming persisted objective on start"
            );
            Some((objective.description.clone(), descriptions))
        }
        Ok(None) => {
            info!("No active objective to resume");
            None
        }
        Err(error) => {
            warn!(error = %error, "Failed to read active objective on start");
            None
        }
    })
}
