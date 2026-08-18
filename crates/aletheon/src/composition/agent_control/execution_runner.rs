//! Host-owned execution and terminal settlement for one supervised Agent.

use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) async fn run_agent(
    kernel: Arc<KernelRuntime>,
    clock: Arc<dyn Clock>,
    repository: Arc<dyn AgentRunProjection>,
    live: Arc<LiveAgentRuns>,
    launcher: Arc<dyn AgentRuntimeLauncher>,
    events: Arc<dyn AgentEventSink>,
    input: AgentRuntimeInput,
    mailbox_bridge: AgentMailboxBridge,
    snapshots: watch::Sender<AgentSnapshot>,
    mut scope: OperationScope,
    mut admission: Box<dyn AgentAdmissionLease>,
    settlement_generation: String,
    settlement_receipts: Arc<dyn SettlementReceiptStore>,
    settlement_metrics: Arc<SettlementMetrics>,
    event_spine: Arc<dyn EventSpine>,
    memory_events: Arc<MemoryRecordingAgentEventSink>,
    lifecycle_hooks: Arc<dyn AgentLifecycleHookSink>,
) {
    let agent = input.handle.agent_id;
    let root_agent = input.handle.root_agent_id;
    let parent_agent = input.handle.parent_agent_id;
    let process = input.handle.process_id;
    let operation = input.handle.operation_id;
    let start = async {
        kernel.signal_process(process, ProcessSignal::Start).await?;
        kernel.start_operation(operation).await?;
        kernel
            .set_active_operation(process, Some(operation))
            .await?;
        anyhow::Ok(())
    }
    .await;
    if let Err(error) = start {
        if let Ok(record) = repository
            .transition(
                agent,
                AgentRunStatus::Queued,
                AgentRunStatus::Failed,
                None,
                Some(error.to_string()),
                clock.wall_now().0,
            )
            .await
        {
            snapshots.send_replace(record.snapshot);
        }
        let _ = kernel
            .cancel_operation(operation, CancelReason::Other("Agent start failed".into()))
            .await;
        let _ = kernel
            .terminate_process(process, ExitReason::Failed(error.to_string()))
            .await;
        let _ = admission.revoke().await;
        live.remove(agent).await;
        return;
    }
    let running = match repository
        .transition(
            agent,
            AgentRunStatus::Queued,
            AgentRunStatus::Running,
            None,
            None,
            clock.wall_now().0,
        )
        .await
    {
        Ok(record) => record,
        Err(error) => {
            let _ = kernel
                .cancel_operation(operation, CancelReason::Other("Agent state failed".into()))
                .await;
            let _ = kernel
                .terminate_process(process, ExitReason::Failed(error.to_string()))
                .await;
            let _ = admission.revoke().await;
            live.remove(agent).await;
            return;
        }
    };
    if let Err(error) = admission.mark_running().await {
        let _ = kernel
            .terminate_process(process, ExitReason::Failed(error.to_string()))
            .await;
        let _ = admission.revoke().await;
        live.remove(agent).await;
        return;
    }
    snapshots.send_replace(running.snapshot);
    lifecycle_hooks
        .emit(agent_lifecycle_hook_context(
            runtime::AgentLifecyclePoint::Start,
            &input.handle,
            &input.request.task,
            input.workspace.as_ref(),
            "running",
        ))
        .await;
    let stop_hook_input = input.clone();
    let wants_reparent = input
        .request
        .background_decls
        .iter()
        .any(|resource| resource.survive_child);

    let (outcome_sender, outcome_receiver) = tokio::sync::oneshot::channel();
    scope.spawn("agent-mailbox", async move {
        match mailbox_bridge.run().await {
            Ok(()) => OperationExitReason::Completed,
            Err(error) => OperationExitReason::Failed(error.message),
        }
    });
    let task_cancel = scope.token();
    scope.spawn("agent-runtime", async move {
        let outcome = tokio::select! {
            _ = task_cancel.cancelled() => Err(control_error(AgentControlErrorKind::Terminal, "Agent runtime cancelled")),
            outcome = launcher.launch(input, events) => outcome,
        };
        let reason = match &outcome {
            Ok(_) => OperationExitReason::Completed,
            Err(error) if error.kind == AgentControlErrorKind::Terminal => {
                OperationExitReason::Cancelled(CancelReason::User)
            }
            Err(error) => OperationExitReason::Failed(error.message.clone()),
        };
        let _ = outcome_sender.send(outcome);
        reason
    });
    let task_exit = scope.join_next().await;
    let outcome = outcome_receiver.await.unwrap_or_else(|_| {
        Err(control_error(
            AgentControlErrorKind::Runtime,
            "Agent runtime task ended without an outcome",
        ))
    });
    let (candidate_status, result, error) = match outcome {
        Ok(result) => (AgentRunStatus::Succeeded, Some(result), None),
        Err(error) if error.kind == AgentControlErrorKind::Terminal => {
            (AgentRunStatus::Cancelled, None, Some(error.message))
        }
        Err(error) => (AgentRunStatus::Failed, None, Some(error.message.clone())),
    };
    // Quiesce the companion mailbox task before any terminal settlement. A
    // successful runtime still cancels the now-obsolete mailbox waiter, then
    // records the scope as normally settled; failures share the abort path.
    scope.cancel();
    let scope_cleanup = if candidate_status == AgentRunStatus::Succeeded {
        scope
            .settle_and_drain(clock.as_ref(), Duration::from_secs(5))
            .await
    } else {
        scope
            .abort_and_drain(clock.as_ref(), Duration::from_secs(5))
            .await
    };
    if scope_cleanup.forced_abort {
        tracing::warn!(
            agent = ?agent,
            exits = ?scope_cleanup.exits,
            "Agent operation scope exceeded its cleanup grace period"
        );
    }
    let settlement_usage = result.as_ref().map(|result| result.usage.clone());
    let terminal_receipt = AgentTerminalReceipt {
        agent_id: agent,
        generation: settlement_generation.clone(),
        status: candidate_status,
        result: result.clone(),
        recorded_at_ms: clock.wall_now().0,
    };
    if let Err(error) = repository.record_terminal_receipt(&terminal_receipt).await {
        tracing::error!(agent = ?agent, %error, "failed to persist host terminal receipt");
    }
    let lease_owner = format!("process:{}", process.0);
    let authoritative_terminal;
    {
        let terminal = match candidate_status {
            AgentRunStatus::Succeeded => ::contracts::SettlementTerminal::Completed,
            AgentRunStatus::Cancelled => ::contracts::SettlementTerminal::Cancelled,
            AgentRunStatus::Failed => ::contracts::SettlementTerminal::Failed {
                reason: error
                    .clone()
                    .unwrap_or_else(|| "Agent runtime failed".into()),
            },
            _ => ::contracts::SettlementTerminal::Recoverable,
        };
        if let Some(live_run) = live.get(agent).await {
            let parent_run = match parent_agent {
                Some(parent) => live.get(parent).await,
                None => None,
            };
            // Both sides are host-minted, spawn-time authority envelopes.  A
            // live parent is also the notification/cancellation route; no
            // model-supplied settlement claim participates in this decision.
            let parent_authority_covers = parent_run.as_ref().is_some_and(|parent| {
                parent
                    .reparent_authority()
                    .covers(live_run.reparent_authority())
            });
            // Static maxima coverage is necessary; the authoritative proof is
            // the atomic BudgetController transfer receipt below.
            let _parent_budget_bounds_cover = parent_run.as_ref().is_some_and(|parent| {
                parent
                    .reparent_authority()
                    .accepts_budget(live_run.reparent_authority())
            });
            let parent_cancellation = parent_run.as_ref().map(|run| run.cancellation.clone());
            let parent_mailbox_target = parent_run.as_ref().map(|run| run.mailbox_target.clone());
            let evidence = Arc::new(SpineSettlementEvidenceSink::new(
                event_spine,
                root_agent.0.to_string(),
                agent.0.to_string(),
                operation,
            ));
            let managed_resources = Arc::new(ManagedSettlementResourcePort::new(
                live_run.clone(),
                parent_authority_covers,
                false,
                parent_cancellation,
                parent_mailbox_target,
            ));
            let engine = SettlementEngine::with_metrics(
                settlement_receipts,
                managed_resources.clone(),
                Arc::new(RepositorySettlementLeasePort::new(repository.clone())),
                evidence,
                settlement_metrics,
            )
            .with_generation(settlement_generation.clone());
            match engine.quiesce(&live_run).await {
                Ok(resources) => {
                    // Closing admission and fixing the resource snapshot must
                    // precede the irreversible budget ownership transfer. A
                    // crash before this point therefore leaves the reservation
                    // wholly child-owned and recoverable.
                    let budget_transfer_receipt = match (parent_agent, settlement_usage.as_ref()) {
                        (Some(parent), Some(usage))
                            if parent_authority_covers && wants_reparent =>
                        {
                            match admission.transfer_remaining_to(parent, usage).await {
                                Ok(receipt) => Some(receipt),
                                Err(error) => {
                                    tracing::warn!(agent = ?agent, %error, "parent budget rejected remaining child reservation");
                                    None
                                }
                            }
                        }
                        _ => None,
                    };
                    managed_resources.set_parent_budget_accepts(budget_transfer_receipt.is_some());
                    let mut terminal =
                        terminal_with_memory_flush(terminal, memory_events.take_error());
                    if budget_transfer_receipt.is_none() {
                        if let Err(error) =
                            settle_admission(&mut *admission, &terminal, settlement_usage.as_ref())
                                .await
                        {
                            tracing::error!(agent = ?agent, %error, "failed to settle Agent admission lease");
                            terminal = ::contracts::SettlementTerminal::Failed {
                                reason: format!(
                                    "Agent admission settlement failed: {}",
                                    error.message
                                ),
                            };
                        }
                    }
                    let request = SettlementRequest {
                        agent_id: agent.0.to_string(),
                        attempt_id: operation.0.to_string(),
                        generation: settlement_generation,
                        old_owner: lease_owner.clone(),
                        parent_owner: parent_agent.map(|parent| format!("agent:{}", parent.0)),
                        terminal,
                        lease_keys: ["admission", "mailbox", "execution"]
                            .into_iter()
                            .map(|label| format!("{label}:{}", agent.0))
                            .collect(),
                        settled_at_ms: clock.wall_now().0,
                    };
                    match engine.settle(request, resources).await {
                        Ok(receipt) => authoritative_terminal = Some(receipt.terminal),
                        Err(error) => {
                            tracing::error!(agent = ?agent, %error, "Agent settlement state machine failed");
                            authoritative_terminal =
                                Some(::contracts::SettlementTerminal::Failed {
                                    reason: format!("Agent settlement failed: {}", error.message),
                                });
                        }
                    }
                }
                Err(error) => {
                    tracing::error!(agent = ?agent, %error, "Agent quiescing failed");
                    for resource in live_run.begin_quiescing().await {
                        let _ = live_run
                            .terminate_managed_resource(
                                &resource.resource_id,
                                &format!("quiesce-failed:{}", resource.resource_id),
                            )
                            .await;
                    }
                    let _ = admission.revoke().await;
                    for label in ["admission", "mailbox", "execution"] {
                        let _ = repository
                            .delete_resource_lease(&format!("{label}:{}", agent.0), &lease_owner)
                            .await;
                    }
                    authoritative_terminal = Some(::contracts::SettlementTerminal::Failed {
                        reason: format!("Agent quiescing failed: {}", error.message),
                    });
                }
            }
        } else {
            let _ = admission.revoke().await;
            for label in ["admission", "mailbox", "execution"] {
                let _ = repository
                    .delete_resource_lease(&format!("{label}:{}", agent.0), &lease_owner)
                    .await;
            }
            authoritative_terminal = Some(::contracts::SettlementTerminal::Failed {
                reason: "Agent settlement lost its live supervised process".into(),
            });
        }
    }
    let authoritative_terminal =
        authoritative_terminal.unwrap_or_else(|| ::contracts::SettlementTerminal::Failed {
            reason: "Agent execution ended without a host terminal receipt".into(),
        });
    let (next, result, error, process_exit) = match authoritative_terminal {
        ::contracts::SettlementTerminal::Completed => (
            AgentRunStatus::Succeeded,
            result,
            None,
            ExitReason::Completed,
        ),
        ::contracts::SettlementTerminal::Cancelled => (
            AgentRunStatus::Cancelled,
            None,
            error.or_else(|| Some("Agent runtime cancelled".into())),
            ExitReason::Cancelled("Agent runtime cancelled".into()),
        ),
        ::contracts::SettlementTerminal::Failed { reason } => (
            AgentRunStatus::Failed,
            None,
            Some(reason.clone()),
            ExitReason::Failed(reason),
        ),
        ::contracts::SettlementTerminal::Recoverable => (
            AgentRunStatus::Interrupted,
            None,
            Some("Agent settlement requires recovery".into()),
            ExitReason::Failed("Agent settlement requires recovery".into()),
        ),
    };
    match next {
        AgentRunStatus::Succeeded => {
            let _ = kernel.succeed_operation(operation).await;
        }
        AgentRunStatus::Cancelled => {
            let _ = kernel.cancel_operation(operation, CancelReason::User).await;
        }
        AgentRunStatus::Failed | AgentRunStatus::Interrupted => {
            let message = error
                .clone()
                .unwrap_or_else(|| "Agent runtime failed".into());
            let _ = kernel.fail_operation(operation, message).await;
        }
        _ => {}
    }
    if let Ok(record) = repository
        .transition(
            agent,
            AgentRunStatus::Running,
            next,
            result,
            error,
            clock.wall_now().0,
        )
        .await
    {
        snapshots.send_replace(record.snapshot);
    } else if let Some(exit) = task_exit {
        tracing::error!(agent = ?agent, reason = ?exit.reason, "failed to persist terminal Agent state");
    }
    lifecycle_hooks
        .emit(agent_lifecycle_hook_context(
            runtime::AgentLifecyclePoint::Stop,
            &stop_hook_input.handle,
            &stop_hook_input.request.task,
            stop_hook_input.workspace.as_ref(),
            match next {
                AgentRunStatus::Succeeded => "succeeded",
                AgentRunStatus::Cancelled => "cancelled",
                AgentRunStatus::Failed => "failed",
                AgentRunStatus::Interrupted => "interrupted",
                _ => "terminal",
            },
        ))
        .await;
    let _ = kernel.terminate_process(process, process_exit).await;
    live.remove(agent).await;
}
