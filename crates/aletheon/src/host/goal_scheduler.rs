//! Policy-free periodic scheduler used by the Goal composition.

use std::future::Future;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

pub async fn run<F, Fut, E>(period: Duration, cancel: CancellationToken, mut tick: F)
where
    F: FnMut(CancellationToken) -> Fut,
    Fut: Future<Output = Result<(), E>>,
    E: std::fmt::Display,
{
    let mut interval = tokio::time::interval(period);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                info!("scheduled task stopped");
                break;
            }
            _ = interval.tick() => {
                if let Err(error) = tick(cancel.child_token()).await {
                    warn!(error = %error, "scheduled task tick failed");
                }
            }
        }
    }
}
