//! External channel construction and supervised polling loops.

use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use ::contracts::ApprovalCategory;
use adapters_sqlite::channel_projection::SqliteChannelProjectionStore;
use adapters_sqlite::ChannelStore;
use crate::wiring::adapters::channel::daemon_adapter::{
    ApprovalRepositoryPort, DaemonChannelApprovalCallbackAdapter, DaemonChannelApprovalExecutor,
    DaemonChannelGoalApplicationPort, DaemonChannelGoalCommandAdapter,
    DaemonChannelTurnApplicationPort, DaemonExternalDraftApprovalExecutor,
};
use crate::wiring::adapters::channel::gmail::GmailGoalDraftCoordinator;
use crate::wiring::adapters::external::GoogleIntegration;
use crate::wiring::application::goal::ObjectiveStore;
use gateway::capability::chat::ChatHandler;
use gateway::capability::greeting::GreetingHandler;
use gateway::external_read::ExternalReadPreprocessor;
use gateway::ports::{ApprovalResolver, ApprovalResolverRegistry};
use gateway::ports::{
    ChannelChatAdapter, ChannelReadApplicationPort, ChannelTurnApplicationPort,
    ExternalAccountDirectory, GoalProgress,
};
use gateway::registry::CapabilityRegistry;
use gateway::router::{ChannelRouter, ChannelTransport};

/// Build the Telegram long-poll channel transport, router, and spawn the
/// poll loop. Returns the task handle for graceful shutdown.
pub(super) fn init_telegram_channel(
    cfg: &crate::config::TelegramChannelConfig,
    data_dir: PathBuf,
    initial_session_id: String,
    orchestrator: Arc<crate::wiring::application::DaemonTurnOrchestrator>,
    objective_store: Arc<Mutex<ObjectiveStore>>,
    approval_repository: Arc<
        std::sync::Mutex<adapters_sqlite::approval_repository::ApprovalRepository>,
    >,
    gmail_goal_drafts: Arc<std::sync::Mutex<GmailGoalDraftCoordinator>>,
    approved_apply: Option<Arc<crate::wiring::application::approval::ApplyCoordinator>>,
    google: Option<Arc<GoogleIntegration>>,
    cancel: CancellationToken,
    goal_progress_rx: Option<mpsc::Receiver<GoalProgress>>,
) -> tokio::task::JoinHandle<()> {
    let store_path = data_dir.join("channels.db");
    let store = ChannelStore::open(&store_path).expect("opening channel store for Telegram");
    let cursor: Option<String> = store.cursor("telegram").unwrap_or(None);

    if let Some(owner_id) = cfg.owner_user_id {
        let external = format!("telegram:{owner_id}");
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
    let transport = gateway::channel::telegram::build_telegram_transport(
        token,
        None,
        poll_timeout,
        cancel.clone(),
    );

    let turn_executor: Arc<dyn ChannelTurnApplicationPort> = Arc::new(
        DaemonChannelTurnApplicationPort::new(orchestrator)
            .with_default_session(::contracts::SessionId(initial_session_id)),
    );

    let goal_executor = Arc::new(DaemonChannelGoalApplicationPort::new(objective_store));
    let approval_conversation = cfg
        .owner_user_id
        .map(|id| gateway::channel::ConversationId(id.to_string()));

    let chat_preprocessor: Option<Arc<dyn ChannelReadApplicationPort>> = google.map(|g| {
        let accounts = g as Arc<dyn ExternalAccountDirectory>;
        Arc::new(ExternalReadPreprocessor::new(accounts)) as Arc<dyn ChannelReadApplicationPort>
    });
    let channel_workspace = ::contracts::WorkspacePolicy::from_resolved_roots(
        std::path::PathBuf::from("/var/lib/aletheon"),
        Vec::new(),
    )
    .expect("the absolute daemon channel workspace is valid");
    let mut registry = CapabilityRegistry::new();
    let mut chat_adapter = ChannelChatAdapter::new(turn_executor, channel_workspace);
    if let Some(read) = chat_preprocessor {
        chat_adapter = chat_adapter.with_read_port(read);
    }
    registry.register(Arc::new(ChatHandler::new(Arc::new(chat_adapter))));
    registry.register(Arc::new(GreetingHandler));

    let approval_adapter = Arc::new(ApprovalRepositoryPort::new(approval_repository));
    let approval_application_port: Arc<dyn gateway::ports::ApprovalApplicationPort> =
        approval_adapter.clone();
    let approval_delivery_port: Arc<dyn gateway::ports::ChannelApprovalDeliveryPort> =
        approval_adapter;
    let gmail_resolver: Arc<dyn ApprovalResolver> =
        Arc::new(DaemonExternalDraftApprovalExecutor::new(gmail_goal_drafts));
    let approval_resolvers = Arc::new(ApprovalResolverRegistry::new());
    approval_resolvers.register(ApprovalCategory::ActivateGoal, gmail_resolver.clone());
    if let Some(coordinator) = approved_apply {
        let resolver: Arc<dyn ApprovalResolver> = Arc::new(DaemonChannelApprovalExecutor::new(
            coordinator,
            ::contracts::ProcessId::new(),
            cancel.clone(),
        ));
        approval_resolvers.set_default(resolver);
    }
    let goal_command_port = Arc::new(DaemonChannelGoalCommandAdapter::new(
        goal_executor,
        Some(gmail_resolver),
    ));
    let approval_callback_port = Arc::new(DaemonChannelApprovalCallbackAdapter::new(
        approval_application_port.clone(),
        approval_resolvers,
    ));
    let router =
        ChannelRouter::with_registry(SqliteChannelProjectionStore::new(store), registry)
            .with_goal_command_port(goal_command_port)
            .with_approval_callback_port(approval_callback_port)
            .with_approval_delivery_port(approval_delivery_port);

    tokio::spawn(async move {
        telegram_poll_loop(
            router,
            transport,
            cursor,
            approval_application_port,
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
    transport: Arc<dyn ChannelTransport>,
    mut cursor: Option<String>,
    approval_application_port: Arc<dyn gateway::ports::ApprovalApplicationPort>,
    approval_conversation: Option<gateway::channel::ConversationId>,
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
            let pending = approval_application_port
                .list_pending(&::contracts::PrincipalId("owner".into()), now_ms);
            match pending {
                Ok(pending) => {
                    for approval in pending {
                        if let Err(error) = router
                            .notify_approval(
                                transport.as_ref(),
                                conversation.clone(),
                                &approval,
                                now_ms,
                            )
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

        if let Err(error) = router.flush_pending_outbox(transport.as_ref(), 100).await {
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
                    match router.process(transport.as_ref(), envelope).await {
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
