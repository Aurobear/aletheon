use super::*;

impl AgentHostAdapter {
    /// Route production child creation through Runtime's typed delegate
    /// command. A composed supervisor is fail-closed when a backend has not
    /// been registered; only fixtures without a supervisor use the fixture-only
    /// entry.
    pub(crate) async fn spawn_via_runtime(
        &self,
        request: AgentSpawnRequest,
    ) -> Result<AgentHandle, AgentControlError> {
        let Some(supervisor) = &self.runtime_agent_supervisor else {
            return self.spawn_fixture(request, None).await;
        };
        let backend = runtime::DelegateBackendId(request.runtime_id.0.clone());
        if supervisor.registry().resolve(&backend).is_none() {
            return Err(control_error(
                AgentControlErrorKind::NotFound,
                format!("Runtime delegate backend is not registered: {}", backend.0),
            ));
        }
        let identity = supervisor
            .spawn_host_request(runtime::DelegateSpawnRequest {
                parent_session: runtime::SessionId(request.root_agent_id.0.to_string()),
                // Child admission is not itself a turn.  Leave the optional
                // parent reference empty instead of fabricating a canonical
                // TurnId from the root AgentId.
                parent_turn: None,
                backend,
                profile: Some(request.profile_id.0.clone()),
                host_request: Some(request),
                command: None,
            })
            .await
            .map_err(|error| {
                control_error(
                    AgentControlErrorKind::Runtime,
                    format!("Runtime Delegate spawn failed: {error}"),
                )
            })?;
        let agent_id = uuid::Uuid::parse_str(&identity.agent_run.0)
            .map(AgentId)
            .map_err(|_| {
                control_error(
                    AgentControlErrorKind::Runtime,
                    "Runtime Delegate receipt was not a Fabric UUID",
                )
            })?;
        self.repository
            .get(agent_id)
            .await?
            .map(|record| record.snapshot.handle)
            .ok_or_else(|| {
                control_error(
                    AgentControlErrorKind::Persistence,
                    "Runtime Delegate spawn completed without an AgentRun receipt",
                )
            })
    }

    pub(crate) async fn spawn_from_runtime(
        &self,
        request: AgentSpawnRequest,
        identity: runtime::DelegateReceipt,
    ) -> Result<AgentHandle, AgentControlError> {
        self.spawn_prepared_with_launcher(request, Some(identity), None)
            .await
    }

    /// Launch a Runtime-admitted child through the backend selected by the
    /// Runtime composition root.  This is the cutover point for extension
    /// backends: the host still owns admission, workspace, mailbox and
    /// settlement preparation, but it no longer re-selects the backend from
    /// the fixture launcher catalog.
    pub(crate) async fn spawn_from_runtime_with_launcher(
        &self,
        request: AgentSpawnRequest,
        identity: runtime::DelegateReceipt,
        launcher: Arc<dyn AgentRuntimeLauncher>,
    ) -> Result<AgentHandle, AgentControlError> {
        self.spawn_prepared_with_launcher(request, Some(identity), Some(launcher))
            .await
    }

    pub(crate) async fn spawn_fixture(
        &self,
        request: AgentSpawnRequest,
        runtime_identity: Option<runtime::DelegateReceipt>,
    ) -> Result<AgentHandle, AgentControlError> {
        self.spawn_prepared_with_launcher(request, runtime_identity, None)
            .await
    }

    async fn spawn_prepared_with_launcher(
        &self,
        mut request: AgentSpawnRequest,
        runtime_identity: Option<runtime::DelegateReceipt>,
        launcher_override: Option<Arc<dyn AgentRuntimeLauncher>>,
    ) -> Result<AgentHandle, AgentControlError> {
        let runtime_admitted = runtime_identity.is_some();
        request.validate()?;
        let launcher = match launcher_override {
            Some(launcher) => launcher,
            None if runtime_admitted => {
                // A Runtime-admitted run must carry the composition-pinned
                // launcher selected by DelegateBackendRegistry. Never fall
                // back to the fixture catalog after identity admission: that
                // would let a reload silently swap the backend generation.
                return Err(control_error(
                    AgentControlErrorKind::Runtime,
                    "Runtime-admitted child has no pinned backend launcher",
                ));
            }
            None => self
                .runtimes
                .as_ref()
                .ok_or_else(|| {
                    control_error(
                        AgentControlErrorKind::NotFound,
                        "Runtime supervisor is not composed",
                    )
                })?
                .resolve(&request.runtime_id)?,
        };
        let mut context_builder = AgentContextProjectionBuilder::new().fork(&request.context)?;
        for reference in &request.broadcast_refs {
            context_builder = context_builder.broadcast_ref(reference.content_id);
        }
        let context = context_builder.build()?;
        let identity = self.validated_parent(&request, runtime_identity).await?;
        // Runtime owns the identity of every admitted run.  A root request
        // carries a caller-chosen correlation id for the admission command,
        // but that id must not survive as the durable root identity after the
        // Runtime receipt is minted.  Keeping the requested id here makes the
        // SQL projection reject the run as a non-root row whose agent id and
        // root id differ.  Child requests retain their authenticated tree
        // root, which is distinct from the newly minted child identity.
        if request.parent_agent_id.is_none() {
            request.root_agent_id = identity.agent_id;
        }
        let (attenuation_report, parent_delegation_authority) =
            if let Some(parent_agent_id) = request.parent_agent_id {
                let parent_authority = if let Some(parent) = self.live.get(parent_agent_id).await {
                    parent.reparent_authority().clone()
                } else {
                    request.delegator_authority.clone().ok_or_else(|| {
                        control_error(
                            AgentControlErrorKind::Forbidden,
                            "non-root Agent spawn has no authenticated delegator authority",
                        )
                    })?
                };
                let parent_profile = identity
                    .parent_profile
                    .as_ref()
                    .and_then(|id| self.resolve_agent_profile(id));
                let child_profile = self.resolve_agent_profile(&request.profile_id);
                if let (Some(parent_profile), Some(child_profile)) =
                    (parent_profile.as_ref(), child_profile.as_ref())
                {
                    if !parent_profile.allows_child(child_profile) {
                        return Err(control_error(
                            AgentControlErrorKind::Forbidden,
                            format!(
                            "child profile '{}' exceeds delegation policy of parent profile '{}'",
                            child_profile.id.0, parent_profile.id.0
                        ),
                        ));
                    }
                }
                let requested = ::contracts::AgentDelegationAuthority::new(
                    request.trusted_workspace.clone(),
                    request.allowed_tools.clone(),
                    request.budget.clone(),
                );
                let (effective, report) = parent_authority.attenuate(&requested)?;
                request.trusted_workspace = effective.workspace;
                request.allowed_tools = effective.allowed_tools;
                request.budget = effective.budget;
                request.validate()?;
                (Some(report), Some(parent_authority))
            } else {
                (None, None)
            };
        admission::constrain_cognitive_workspace(&mut request)?;
        request.validate()?;
        // Derive re-delegation only after role-specific workspace narrowing.
        // Otherwise a cognitively scoped child could retain authority to mint
        // descendants against its parent's broader workspace.
        let delegated_tools = self
            .resolve_agent_profile(&request.profile_id)
            .map(|profile| profile.delegated_tools.clone())
            .unwrap_or_else(|| request.allowed_tools.clone());
        let requested_delegation = ::contracts::AgentDelegationAuthority::new(
            request.trusted_workspace.clone(),
            delegated_tools,
            request.budget.clone(),
        );
        let child_delegation_authority = match parent_delegation_authority {
            Some(parent) => parent.attenuate(&requested_delegation)?.0,
            None => requested_delegation,
        };
        let agent_id = identity.agent_id;
        let workspace_id = agent_workspace_id(agent_id);
        let request_hash = agent_spawn_request_hash(&request)?;
        let requirements = launcher
            .resource_requirements()
            .validate()
            .map_err(|message| control_error(AgentControlErrorKind::Capacity, message))?;
        let storage = AgentStorageRequest {
            bytes: requirements.storage_bytes,
            items: requirements.storage_items,
        };
        let mut admission = self
            .admission
            .reserve(AgentAdmissionRequest::new_for_agent(
                agent_id,
                &request,
                identity.depth,
                identity.parent_profile.as_ref(),
                storage,
            ))
            .await?;

        let deadline = Some(::contracts::MonoDeadline::after(
            self.clock.mono_now(),
            request.budget.max_elapsed_ms,
        ));
        let process = match self
            .kernel
            .spawn_process(SpawnSpec {
                agent_id,
                parent: request.parent_process_id,
                profile: request.profile_id.clone(),
                namespace: NamespaceId(request.root_agent_id.0.to_string()),
                initial_operation: None,
                deadline,
                ownership: ::contracts::ProcessOwnership::ThreadBackground {
                    thread_id: ::contracts::ThreadId(request.root_agent_id.0.to_string()),
                },
            })
            .await
        {
            Ok(process) => process,
            Err(error) => {
                let _ = admission.revoke().await;
                return Err(runtime_error(error));
            }
        };
        let process_snapshot = match self.kernel.inspect_process(process.id).await {
            Ok(snapshot) => snapshot,
            Err(error) => {
                let _ = self
                    .kernel
                    .terminate_process(process.id, ExitReason::Failed(error.to_string()))
                    .await;
                let _ = admission.revoke().await;
                return Err(runtime_error(error));
            }
        };
        let root_process_id = identity.root_process_id.unwrap_or(process.id);
        let root_workspace_id = identity
            .root_workspace_id
            .unwrap_or_else(|| workspace_id.clone());
        self.kernel.upsert_space_binding(
            process_snapshot.space,
            ContextBinding::Agora(workspace_id.clone(), AgoraVersion(0)),
        );
        if let Err(error) = self.kernel.set_space_overlay(
            process_snapshot.space,
            "agent.workspace_receipt",
            serde_json::json!({
                "workspace_id": workspace_id,
                "root_process_id": root_process_id,
                "broadcast_refs": request.broadcast_refs,
            }),
        ) {
            let _ = self
                .kernel
                .terminate_process(process.id, ExitReason::Failed(error.to_string()))
                .await;
            let _ = admission.revoke().await;
            return Err(runtime_error(error));
        }
        let operation = match self
            .kernel
            .submit_operation(OperationRequest {
                owner: process.id,
                parent: None,
                kind: OperationKind::SubAgent,
                deadline,
            })
            .await
        {
            Ok(operation) => operation,
            Err(error) => {
                let _ = self
                    .kernel
                    .terminate_process(process.id, ExitReason::Failed(error.to_string()))
                    .await;
                let _ = admission.revoke().await;
                return Err(runtime_error(error));
            }
        };
        let mailbox_target = Target::from(format!("agent:{}", agent_id.0));
        let mailbox: Arc<dyn Mailbox> = Arc::new(InProcessMailbox::with_capacity(MAILBOX_CAPACITY));
        if let Err(error) = self
            .register_agent_mailbox(process.id, mailbox_target.clone(), mailbox.clone())
            .await
        {
            let _ = self
                .kernel
                .cancel_operation(
                    operation.id,
                    CancelReason::Other("mailbox setup failed".into()),
                )
                .await;
            let _ = self
                .kernel
                .terminate_process(process.id, ExitReason::Failed(error.to_string()))
                .await;
            let _ = admission.revoke().await;
            return Err(runtime_error(error));
        }

        let handle = AgentHandle {
            agent_id,
            root_agent_id: request.root_agent_id,
            parent_agent_id: request.parent_agent_id,
            process_id: process.id,
            operation_id: operation.id,
            runtime_id: request.runtime_id.clone(),
            profile_id: request.profile_id.clone(),
        };
        let parent_projection_receipt = context_projection_receipt(&context)?;
        let memory_context = mnemosyne::AgentMemoryContext::verified(
            process.id,
            agent_id,
            ::contracts::AgentTaskId(format!("task:{request_hash}")),
            parent_projection_receipt,
        )
        .map_err(|error| AgentControlError::invalid(error.to_string()))?;
        self.agent_memory_vault
            .register(&memory_context)
            .map_err(|error| {
                control_error(AgentControlErrorKind::Persistence, error.to_string())
            })?;
        let created_at_ms = self.clock.wall_now().0;
        let queued = AgentSnapshot {
            handle: handle.clone(),
            status: AgentRunStatus::Queued,
            result: None,
            created_at_ms,
            started_at_ms: None,
            ended_at_ms: None,
            last_error: None,
        };
        let record = AgentRunRecord {
            snapshot: queued.clone(),
            request: request.clone(),
            request_hash,
            workspace_id: workspace_id.clone(),
            root_process_id,
            broadcast_refs: request.broadcast_refs.clone(),
            version: 0,
            retain_until_ms: created_at_ms.saturating_add(DEFAULT_RETENTION_MS),
            resumability: launcher.resumability(),
            recovery: None,
        };
        if let Err(error) = self.repository.create(&record).await {
            let _ = self
                .kernel
                .cancel_operation(
                    operation.id,
                    CancelReason::Other("Agent persistence failed".into()),
                )
                .await;
            let _ = self
                .kernel
                .terminate_process(process.id, ExitReason::Failed(error.to_string()))
                .await;
            let _ = admission.revoke().await;
            return Err(error);
        }
        let lease_owner = format!("process:{}", process.id.0);
        let lease_expiry = created_at_ms.saturating_add(request.budget.max_elapsed_ms as i64);
        for (kind, label) in [
            (AgentResourceLeaseKind::Admission, "admission"),
            (AgentResourceLeaseKind::Mailbox, "mailbox"),
            (AgentResourceLeaseKind::Execution, "execution"),
        ] {
            self.repository
                .put_resource_lease(&AgentResourceLease {
                    lease_key: format!("{label}:{}", agent_id.0),
                    agent_id,
                    kind,
                    owner: lease_owner.clone(),
                    expires_at_ms: lease_expiry,
                    worktree_root: None,
                    worktree_path: None,
                    expected_head: None,
                })
                .await?;
        }

        let scope = OperationScope::new(operation.id);
        let cancellation = scope.token();
        let (mailbox_bridge, inbox) =
            AgentMailboxBridge::bounded(mailbox, MAILBOX_CAPACITY, cancellation.clone())?;
        let (snapshots, _) = watch::channel(queued);
        let live_run = LiveAgentRun::new(
            snapshots.clone(),
            mailbox_target,
            cancellation.clone(),
            request.background_decls.clone(),
            ReparentAuthority::new(
                child_delegation_authority.workspace.clone(),
                child_delegation_authority.allowed_tools.clone(),
                child_delegation_authority.budget.clone(),
            ),
        )?;
        let mut background_cancellations = std::collections::HashMap::new();
        let mut background_registrations = std::collections::HashMap::new();
        let mut background_notification_targets = std::collections::HashMap::new();
        for declaration in &request.background_decls {
            let token = live_run
                .resource_cancellation(&declaration.resource_id)
                .await
                .ok_or_else(|| {
                    control_error(
                        AgentControlErrorKind::Runtime,
                        "reviewed background resource has no managed cancellation token",
                    )
                })?;
            background_cancellations.insert(declaration.resource_id.clone(), token);
            let registration = live_run
                .resource_registration(&declaration.resource_id)
                .ok_or_else(|| {
                    control_error(
                        AgentControlErrorKind::Runtime,
                        "reviewed background resource has no producer registration",
                    )
                })?;
            background_registrations.insert(declaration.resource_id.clone(), registration);
            if let Some(target) = live_run.notification_target(&declaration.resource_id) {
                background_notification_targets.insert(declaration.resource_id.clone(), target);
            }
        }
        let inserted = self.live.insert(agent_id, live_run).await;
        if !inserted {
            let _ = self
                .kernel
                .cancel_operation(
                    operation.id,
                    CancelReason::Other("duplicate live Agent".into()),
                )
                .await;
            let _ = self
                .kernel
                .terminate_process(
                    process.id,
                    ExitReason::Failed("duplicate live Agent".into()),
                )
                .await;
            let _ = admission.revoke().await;
            return Err(control_error(
                AgentControlErrorKind::Conflict,
                "Agent already has a live runtime",
            ));
        }

        // This is the two-phase launch boundary: the process identity exists
        // and all cancellation/recovery state is durable, but the runtime has
        // not been scheduled. A cognitive worker receives workspace authority
        // only after its exact Agora task handoff commits here.
        if let Some(binding) = request.cognitive_binding.clone() {
            let binding_result = match &self.cognitive_task_admission {
                Some(port) => port.bind_before_launch(binding, process.id).await,
                None => Err(control_error(
                    AgentControlErrorKind::Forbidden,
                    "cognitive task binding requested without an admission port",
                )),
            };
            if let Err(error) = binding_result {
                let _ = self
                    .control_cancel(request.root_agent_id, handle.agent_id)
                    .await;
                return Err(error);
            }
        }

        let kernel = self.kernel.clone();
        let clock = self.clock.clone();
        let repository = self.repository.clone();
        let live = self.live.clone();
        let events = self.events.clone();
        let event_spine = self.event_spine.clone();
        let event_projections = self.event_projections.clone();
        let settlement_generation = self.settlement_generation.clone();
        let settlement_receipts = self.settlement_receipts.clone();
        let settlement_metrics = self.settlement_metrics.clone();
        let lifecycle_hooks = self.lifecycle_hooks.clone();
        let runtime_agent_supervisor = self.runtime_agent_supervisor.clone();
        // Carry the Runtime admission receipt through the validated identity;
        // do not keep a second Aletheon-side generation map.
        let runtime_agent_generation = identity
            .runtime_generation
            .clone()
            .unwrap_or(runtime::Generation(1));
        if !runtime_admitted {
            if let Some(supervisor) = &runtime_agent_supervisor {
                if let Err(error) = supervisor
                    .record_started(
                        runtime::SessionId(handle.root_agent_id.0.to_string()),
                        runtime::AgentRunId(handle.agent_id.0.to_string()),
                        runtime_agent_generation.clone(),
                        Some(handle.runtime_id.0.clone()),
                    )
                    .await
                {
                    let _ = self
                        .control_cancel(handle.root_agent_id, handle.agent_id)
                        .await;
                    return Err(control_error(
                        AgentControlErrorKind::Persistence,
                        format!("Runtime Agent admission journal failed: {error}"),
                    ));
                }
            }
        }
        let runtime_input = AgentRuntimeInput {
            workspace: request.trusted_workspace.clone(),
            delegation_authority: child_delegation_authority,
            request,
            handle: handle.clone(),
            workspace_id,
            root_workspace_id,
            root_process_id,
            context,
            memory_context: memory_context.clone(),
            inbox,
            cancellation,
            runtime_process: Arc::new(DurableRuntimeProcessRegistration {
                kernel: kernel.clone(),
                repository: repository.clone(),
                agent_id: handle.agent_id,
                process_id: handle.process_id,
            }),
            background_cancellations,
            background_registrations,
            background_notification_targets,
        };
        let events: Arc<dyn AgentEventSink> = Arc::new(SpineAgentEventSink::new(
            events,
            event_spine.clone(),
            runtime_input.clone(),
            event_projections,
        ));
        let mut memory_events = MemoryRecordingAgentEventSink::new(
            events,
            self.agent_memory_vault.clone(),
            memory_context,
        );
        if let Some(memory) = &self.durable_memory {
            memory_events = memory_events.with_durable_memory(memory.clone());
        }
        let memory_events = Arc::new(memory_events);
        let events: Arc<dyn AgentEventSink> = if let Some(supervisor) = runtime_agent_supervisor {
            Arc::new(RuntimeAgentStreamAdapter::new(
                memory_events.clone(),
                supervisor,
                runtime::AgentRunId(handle.agent_id.0.to_string()),
            ))
        } else {
            memory_events.clone()
        };
        if let Some(report) = attenuation_report {
            events
                .emit(AgentRuntimeEvent::CapabilityAttenuated {
                    agent_id: handle.agent_id,
                    process_id: handle.process_id,
                    operation_id: handle.operation_id,
                    report,
                })
                .await;
        }
        self.tasks.lock().await.spawn(async move {
            run_agent(
                kernel,
                clock,
                repository,
                live,
                launcher,
                events,
                runtime_input,
                mailbox_bridge,
                snapshots,
                scope,
                admission,
                settlement_generation,
                settlement_receipts,
                settlement_metrics,
                event_spine,
                memory_events,
                lifecycle_hooks,
            )
            .await;
        });
        Ok(handle)
    }
}
