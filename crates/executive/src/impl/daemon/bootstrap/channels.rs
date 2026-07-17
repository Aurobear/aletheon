//! External channel construction and supervised polling loops.

use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::r#impl::channel::daemon_adapter::{
    DaemonChannelApprovalExecutor, DaemonChannelGoalExecutor, DaemonChannelTurnExecutor,
    DaemonGmailDraftApprovalExecutor,
};
use crate::r#impl::channel::gmail::GmailGoalDraftCoordinator;
use crate::r#impl::channel::router::{
    ChannelApprovalExecutor, ChannelGoalExecutor, ChannelRouter, ChannelTransport,
    ChannelTurnExecutor, GmailDraftApprovalExecutor, GoalProgress,
};
use crate::r#impl::channel::store::ChannelStore;
use crate::r#impl::channel::telegram::TelegramTransport;
use crate::r#impl::external::GoogleIntegration;
use crate::r#impl::goal::ObjectiveStore;

/// Build the Telegram long-poll channel transport, router, and spawn the
/// poll loop. Returns the task handle for graceful shutdown.
pub(super) fn init_telegram_channel(
    cfg: &cognit::config::TelegramConfig,
    data_dir: PathBuf,
    orchestrator: Arc<crate::service::DaemonTurnOrchestrator>,
    objective_store: Arc<Mutex<ObjectiveStore>>,
    approval_repository: Arc<std::sync::Mutex<crate::r#impl::approval::ApprovalRepository>>,
    gmail_goal_drafts: Arc<std::sync::Mutex<GmailGoalDraftCoordinator>>,
    approved_apply: Option<Arc<crate::r#impl::approval::ApplyCoordinator>>,
    google: Option<Arc<GoogleIntegration>>,
    cancel: CancellationToken,
    goal_progress_rx: Option<mpsc::Receiver<GoalProgress>>,
) -> tokio::task::JoinHandle<()> {
    let store_path = data_dir.join("channels.db");
    let store = ChannelStore::open(&store_path).expect("opening channel store for Telegram");
    let cursor: Option<String> = store.cursor("telegram").unwrap_or(None);

    if let Some(owner_id) = cfg.owner_user_id {
        let external = format!("telegram:{}", owner_id);
        store
            .bind("telegram", &external, "owner", "active")
            .expect("binding Telegram owner");
        info!(owner_id = owner_id, "Telegram owner binding seeded");
    } else {
        warn!("Telegram enabled but owner_user_id not set");
    }

    let token = cfg
        .bot_token_env
        .as_ref()
        .and_then(|env_name| std::env::var(env_name).ok())
        .unwrap_or_default();
    if token.is_empty() {
        warn!(
            env = ?cfg.bot_token_env,
            "Telegram bot token not found in environment"
        );
    }

    let poll_timeout = cfg.poll_timeout_secs.clamp(1, 50);
    let transport = TelegramTransport::new(token, None, poll_timeout, cancel.clone());

    let turn_executor: Arc<dyn ChannelTurnExecutor> =
        Arc::new(DaemonChannelTurnExecutor::new(orchestrator));

    let goal_executor: Arc<dyn ChannelGoalExecutor> =
        Arc::new(DaemonChannelGoalExecutor::new(objective_store));
    let approval_repository_for_poll = approval_repository.clone();
    let approval_conversation = cfg
        .owner_user_id
        .map(|id| fabric::channel::ConversationId(id.to_string()));
    let mut router = ChannelRouter::new(store, turn_executor)
        .with_goal_executor(goal_executor)
        .with_approval_repository(approval_repository);
    let gmail_executor: Arc<dyn GmailDraftApprovalExecutor> =
        Arc::new(DaemonGmailDraftApprovalExecutor::new(gmail_goal_drafts));
    router = router.with_gmail_draft_executor(gmail_executor);
    if let Some(google) = google {
        router = router.with_google_accounts(google);
    }
    if let Some(coordinator) = approved_apply {
        let executor: Arc<dyn ChannelApprovalExecutor> =
            Arc::new(DaemonChannelApprovalExecutor::new(
                coordinator,
                fabric::ProcessId::new(),
                cancel.clone(),
            ));
        router = router.with_approval_executor(executor);
    }

    tokio::spawn(async move {
        telegram_poll_loop(
            router,
            transport,
            cursor,
            approval_repository_for_poll,
            approval_conversation,
            cancel,
            goal_progress_rx,
        )
        .await;
    })
}

/// Long-poll loop with jittered exponential backoff and cancellation.
async fn telegram_poll_loop(
    mut router: ChannelRouter,
    transport: TelegramTransport,
    mut cursor: Option<String>,
    approval_repository: Arc<std::sync::Mutex<crate::r#impl::approval::ApprovalRepository>>,
    approval_conversation: Option<fabric::channel::ConversationId>,
    cancel: CancellationToken,
    mut goal_progress_rx: Option<mpsc::Receiver<GoalProgress>>,
) {
    let mut backoff_ms: u64 = 1_000;
    let max_backoff_ms: u64 = 60_000;

    loop {
        if cancel.is_cancelled() {
            info!("Telegram poll loop exited (cancel token fired)");
            break;
        }

        if let Some(conversation) = &approval_conversation {
            let now_ms = chrono::Utc::now().timestamp_millis();
            let pending = approval_repository
                .lock()
                .unwrap()
                .list_pending(&fabric::PrincipalId("owner".into()), now_ms);
            match pending {
                Ok(pending) => {
                    for approval in pending {
                        if let Err(error) = router
                            .notify_approval(&transport, conversation.clone(), &approval, now_ms)
                            .await
                        {
                            warn!(approval_id = %approval.id, error = %error, "Telegram approval notification failed");
                        }
                    }
                }
                Err(error) => {
                    warn!(error = %error, "Loading pending Telegram approvals failed")
                }
            }
        }

        if let Err(error) = router.flush_pending_outbox(&transport, 100).await {
            warn!(error = %error, "Flushing durable Telegram outbox failed");
        }

        let result = tokio::select! {
            _ = cancel.cancelled() => {
                info!("Telegram poll loop cancelled during receive wait");
                break;
            }
            r = transport.receive(cursor.clone()) => r,
            progress = async {
                match &mut goal_progress_rx {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                if let (Some(progress), Some(conversation)) = (progress, &approval_conversation) {
                    if let Err(error) = router.queue_goal_progress(
                        transport.channel_id(),
                        conversation.clone(),
                        &progress,
                    ) {
                        warn!(goal_id = %progress.goal_id, error = %error, "Telegram Goal progress notification failed");
                    }
                }
                continue;
            }
        };

        match result {
            Ok(envelopes) => {
                backoff_ms = 1_000;
                if envelopes.is_empty() {
                    continue;
                }
                let mut sorted: Vec<_> = envelopes;
                sorted.sort_by_key(|e| e.message.message_id.0.parse::<i64>().unwrap_or(0));
                for envelope in sorted {
                    let next_cursor = envelope.next_cursor.clone();
                    match router.process(&transport, envelope).await {
                        Ok(()) => {
                            cursor = Some(next_cursor);
                        }
                        Err(e) => {
                            warn!(error = %e, "Telegram router process failed");
                            break;
                        }
                    }
                }
            }
            Err(e) => {
                warn!(error = %e.to_string(), backoff_ms, "Telegram receive error, backing off");
                if cancel.is_cancelled() {
                    break;
                }
                let jitter_ns = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.subsec_nanos())
                    .unwrap_or(0);
                let jitter_ms = (backoff_ms / 4).saturating_mul(jitter_ns as u64 % 101 / 100);
                tokio::time::sleep(std::time::Duration::from_millis(backoff_ms + jitter_ms)).await;
                backoff_ms = (backoff_ms * 2).min(max_backoff_ms);
            }
        }
    }
}
