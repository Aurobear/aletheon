//! Deterministic executable specification of the hardware safety boundary.

use crate::{
    CommandDecision, CommandReceipt, CommandSequence, ControlLease, ControlPermit, DeviceClass,
    DeviceId, DeviceManifest, DeviceNamespace, DeviceProvider, MonotonicClock, OperationId,
    RejectionReason, SafetyState, TelemetryEnvelope, TypedCommand, ValidatedCommand,
};
use std::collections::BTreeSet;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

pub struct SimulatedDevice {
    pub manifest: DeviceManifest,
    position: (f64, f64),
    battery: f64,
    lease: Option<ControlLease>,
    clock: Arc<dyn MonotonicClock>,
    safety: SafetyState,
    last_sequence: Option<CommandSequence>,
    telemetry_sequence: u64,
    connected: bool,
}

impl SimulatedDevice {
    pub fn mobile_robot(id: &str, clock: Arc<dyn MonotonicClock>) -> Self {
        Self {
            manifest: DeviceManifest {
                id: DeviceId(id.into()),
                class: DeviceClass::Robot,
                namespace: DeviceNamespace::Simulation,
                model: "sim-mobile-v1".into(),
                capabilities: BTreeSet::from(["navigate".into(), "stop".into()]),
                firmware: Some("sim-0.1.0".into()),
            },
            position: (0.0, 0.0),
            battery: 1.0,
            lease: None,
            clock,
            safety: SafetyState::Ready,
            last_sequence: None,
            telemetry_sequence: 0,
            connected: true,
        }
    }
    pub fn safety(&self) -> SafetyState {
        self.safety
    }
    pub fn position(&self) -> (f64, f64) {
        self.position
    }
    pub fn grant_lease(&mut self, lease: ControlLease) -> Result<(), String> {
        if lease.device != self.manifest.id {
            return Err("lease device mismatch".into());
        }
        self.lease = Some(lease);
        Ok(())
    }
    pub fn disconnect(&mut self) {
        self.connected = false;
        let _ = self.safe_stop();
    }
    pub fn fault(&mut self) {
        self.safety = SafetyState::Faulted;
        self.position = (0.0, 0.0);
    }
    pub fn enforce_expiry(&mut self) {
        if self
            .lease
            .as_ref()
            .is_some_and(|lease| self.clock.now() >= lease.expires_at)
        {
            let _ = self.safe_stop();
        }
    }
    pub fn telemetry(&mut self, operation: Option<OperationId>) -> TelemetryEnvelope {
        self.telemetry_sequence = self.telemetry_sequence.saturating_add(1);
        TelemetryEnvelope {
            device: self.manifest.id.clone(),
            operation,
            stream: "pose".into(),
            sequence: self.telemetry_sequence,
            source_time: self.clock.now(),
            safety: self.safety,
            payload: serde_json::json!({"x":self.position.0,"y":self.position.1,"battery":self.battery}),
        }
    }

    pub fn execute(
        &mut self,
        command: &TypedCommand,
        permit: Option<&ControlPermit>,
    ) -> CommandReceipt {
        let now = self.clock.now();
        let before = self.safety;
        let reject = |this: &mut Self, reason: RejectionReason, fail_safe: bool| {
            if fail_safe {
                let _ = this.safe_stop();
            }
            CommandReceipt {
                operation: command.operation.clone(),
                principal: command.principal.clone(),
                device: command.device.clone(),
                command_id: command.command_id.clone(),
                sequence: command.sequence,
                decision: CommandDecision::Rejected(reason),
                safety_before: before,
                safety_after: this.safety,
                observed_at: now,
            }
        };
        let Some(permit) = permit else {
            return reject(self, RejectionReason::MissingPermit, false);
        };
        if permit.revoked {
            return reject(self, RejectionReason::RevokedPermit, false);
        }
        if now >= permit.expires_at {
            return reject(self, RejectionReason::ExpiredPermit, false);
        }
        if permit.operation != command.operation {
            return reject(self, RejectionReason::PermitOperationMismatch, false);
        }
        if permit.principal != command.principal {
            return reject(self, RejectionReason::PermitPrincipalMismatch, false);
        }
        if permit.device != command.device {
            return reject(self, RejectionReason::PermitDeviceMismatch, false);
        }
        if !permit.scope.contains(&command.schema) {
            return reject(self, RejectionReason::PermitOutOfScope, false);
        }
        if command.device != self.manifest.id {
            return reject(self, RejectionReason::WrongDevice, false);
        }
        if !self.manifest.capabilities.contains(&command.schema) {
            return reject(self, RejectionReason::SchemaMismatch, false);
        }
        if now >= command.deadline {
            return reject(self, RejectionReason::ExpiredDeadline, true);
        }
        if self
            .last_sequence
            .is_some_and(|sequence| command.sequence <= sequence)
        {
            return reject(self, RejectionReason::ReplayOrOutOfOrder, false);
        }
        if self.safety == SafetyState::Faulted && !command.is_stop() {
            return reject(self, RejectionReason::UnsafeState, false);
        }

        if !command.is_stop() {
            let Some(lease) = self.lease.as_ref() else {
                return reject(self, RejectionReason::MissingLease, false);
            };
            if now >= lease.expires_at {
                return reject(self, RejectionReason::ExpiredLease, true);
            }
            if lease.holder != command.principal {
                return reject(self, RejectionReason::WrongHolder, false);
            }
            if lease.device != command.device {
                return reject(self, RejectionReason::WrongDevice, false);
            }
            if lease.operation != command.operation {
                return reject(self, RejectionReason::LeaseOperationMismatch, false);
            }
            if !lease.scope.contains(&command.schema) {
                return reject(self, RejectionReason::LeaseOutOfScope, false);
            }
        }

        let decision = if command.is_stop() {
            let _ = self.safe_stop();
            CommandDecision::FailSafeApplied
        } else if self.apply(ValidatedCommand(command)).is_err() {
            return reject(self, RejectionReason::InvalidPayload, true);
        } else {
            CommandDecision::Accepted
        };
        self.last_sequence = Some(command.sequence);
        CommandReceipt {
            operation: command.operation.clone(),
            principal: command.principal.clone(),
            device: command.device.clone(),
            command_id: command.command_id.clone(),
            sequence: command.sequence,
            decision,
            safety_before: before,
            safety_after: self.safety,
            observed_at: now,
        }
    }
}

impl DeviceProvider for SimulatedDevice {
    type Error = String;
    fn apply(&mut self, validated: ValidatedCommand<'_>) -> Result<(), Self::Error> {
        let command = validated.command();
        if !self.connected {
            self.safe_stop()?;
            return Err("device disconnected".into());
        }
        let x = command
            .payload
            .get("x")
            .and_then(serde_json::Value::as_f64)
            .ok_or("navigate x missing")?;
        let y = command
            .payload
            .get("y")
            .and_then(serde_json::Value::as_f64)
            .ok_or("navigate y missing")?;
        self.position = (x, y);
        self.safety = SafetyState::Active;
        Ok(())
    }
    fn safe_stop(&mut self) -> Result<(), Self::Error> {
        if self.safety != SafetyState::Faulted {
            self.safety = SafetyState::Stopping;
            self.position = (0.0, 0.0);
            self.safety = SafetyState::SafeStopped;
        }
        Ok(())
    }
}

/// Async provider facade over the deterministic device safety boundary.
pub struct SimulatedEmbodiment {
    inner: tokio::sync::Mutex<SimulatedDevice>,
    sequence: AtomicU64,
}

impl SimulatedEmbodiment {
    pub fn mobile_robot(id: &str, clock: Arc<dyn MonotonicClock>) -> Self {
        Self {
            inner: tokio::sync::Mutex::new(SimulatedDevice::mobile_robot(id, clock)),
            sequence: AtomicU64::new(0),
        }
    }
}

#[async_trait::async_trait]
impl crate::EmbodimentProvider for SimulatedEmbodiment {
    async fn observe(
        &self,
        device: &fabric::DeviceId,
    ) -> Result<Vec<fabric::EmbodiedObservation>, crate::ProviderError> {
        Ok(vec![self
            .get_state(device)
            .await?
            .expect("simulator always exposes state")])
    }

    async fn get_state(
        &self,
        device: &fabric::DeviceId,
    ) -> Result<Option<fabric::EmbodiedObservation>, crate::ProviderError> {
        let mut guard = self.inner.lock().await;
        if guard.manifest.id != *device {
            return Err(crate::ProviderError::Rejected("device mismatch".into()));
        }
        let telemetry = guard.telemetry(None);
        let time = fabric::MonoTime(telemetry.source_time.0);
        Ok(Some(fabric::EmbodiedObservation {
            schema: telemetry.stream,
            schema_version: 1,
            source: format!("sim:{}", telemetry.device.0),
            sequence: telemetry.sequence,
            source_time: time,
            received_at: time,
            valid_until: Some(fabric::MonoDeadline::after(time, 1_000)),
            confidence: 1.0,
            frame_ref: Some("map".into()),
            payload: telemetry.payload,
            evidence: vec![],
        }))
    }

    async fn list_skills(
        &self,
        device: &fabric::DeviceId,
    ) -> Result<Vec<fabric::SkillDescriptor>, crate::ProviderError> {
        let guard = self.inner.lock().await;
        if guard.manifest.id != *device {
            return Err(crate::ProviderError::Rejected("device mismatch".into()));
        }
        Ok(vec![fabric::SkillDescriptor {
            skill: fabric::SkillId("navigate".into()),
            device: device.clone(),
            summary: "navigate to a two-dimensional target".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {"x": {"type": "number"}, "y": {"type": "number"}},
                "required": ["x", "y"],
                "additionalProperties": false
            }),
            risk: fabric::RiskClass::Medium,
            timeout_ms: 30_000,
            cancellable: true,
            preconditions: vec!["active control lease".into()],
            success_criteria: vec!["simulated position equals target".into()],
        }])
    }

    async fn execute_skill(
        &self,
        command: crate::ValidatedSkillCommand<'_>,
        progress: Arc<dyn crate::SkillProgressSink>,
    ) -> Result<fabric::SkillResult, crate::ProviderError> {
        let request = command.request();
        if request.skill.0 != "navigate" {
            return Err(crate::ProviderError::Rejected(format!(
                "unsupported skill {}",
                request.skill.0
            )));
        }
        let x = request
            .parameters
            .get("x")
            .and_then(serde_json::Value::as_f64)
            .ok_or_else(|| crate::ProviderError::Rejected("navigate x missing".into()))?;
        let y = request
            .parameters
            .get("y")
            .and_then(serde_json::Value::as_f64)
            .ok_or_else(|| crate::ProviderError::Rejected("navigate y missing".into()))?;
        let operation_id = command
            .permit()
            .operation
            .0
            .parse()
            .map_err(|_| crate::ProviderError::Rejected("invalid operation mapping".into()))?;
        let sequence = crate::CommandSequence(self.sequence.fetch_add(1, Ordering::SeqCst) + 1);
        let typed = crate::TypedCommand {
            command_id: format!("skill-{}", sequence.0),
            operation: command.permit().operation.clone(),
            principal: command.permit().principal.clone(),
            sequence,
            device: request.device.clone(),
            schema: request.skill.0.clone(),
            payload: serde_json::json!({"x": x, "y": y}),
            deadline: command.permit().expires_at,
        };
        let mut guard = self.inner.lock().await;
        guard
            .grant_lease(command.lease().clone())
            .map_err(crate::ProviderError::Rejected)?;
        let receipt = guard.execute(&typed, Some(command.permit()));
        let outcome = if receipt.accepted() {
            fabric::SkillOutcome::Succeeded
        } else {
            fabric::SkillOutcome::Failed {
                reason: format!("{:?}", receipt.decision),
            }
        };
        if !receipt.accepted() {
            return Ok(fabric::SkillResult {
                operation_id,
                skill: request.skill.clone(),
                device: request.device.clone(),
                outcome,
                duration_ms: 0,
                evidence: vec![],
            });
        }
        progress
            .progress(fabric::SkillProgress {
                operation_id,
                skill: request.skill.clone(),
                fraction: 1.0,
                note: "navigate settled".into(),
                at: fabric::MonoTime(receipt.observed_at.0),
            })
            .await;
        Ok(fabric::SkillResult {
            operation_id,
            skill: request.skill.clone(),
            device: request.device.clone(),
            outcome,
            duration_ms: 0,
            evidence: vec![],
        })
    }

    async fn cancel(
        &self,
        device: &fabric::DeviceId,
        _operation: &crate::OperationId,
    ) -> Result<crate::CancelAck, crate::ProviderError> {
        let mut guard = self.inner.lock().await;
        if guard.manifest.id != *device {
            return Err(crate::ProviderError::Rejected("device mismatch".into()));
        }
        guard.safe_stop().map_err(crate::ProviderError::Rejected)?;
        Ok(crate::CancelAck {
            device: device.clone(),
        })
    }

    async fn safe_stop(
        &self,
        device: &fabric::DeviceId,
    ) -> Result<crate::StopReceipt, crate::ProviderError> {
        let mut guard = self.inner.lock().await;
        if guard.manifest.id != *device {
            return Err(crate::ProviderError::Rejected("device mismatch".into()));
        }
        guard.safe_stop().map_err(crate::ProviderError::Rejected)?;
        Ok(crate::StopReceipt {
            device: device.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ManualClock, MonotonicInstant, PrincipalId};
    fn setup() -> (
        Arc<ManualClock>,
        SimulatedDevice,
        ControlPermit,
        TypedCommand,
    ) {
        let clock = Arc::new(ManualClock::new(10));
        let mut robot = SimulatedDevice::mobile_robot("bot", clock.clone());
        let operation = OperationId("op".into());
        let principal = PrincipalId("alice".into());
        robot
            .grant_lease(ControlLease {
                lease_id: "l".into(),
                operation: operation.clone(),
                device: DeviceId("bot".into()),
                holder: principal.clone(),
                scope: BTreeSet::from(["navigate".into()]),
                expires_at: MonotonicInstant(50),
                exclusive: true,
            })
            .unwrap();
        let permit = ControlPermit {
            permit_id: "p".into(),
            operation: operation.clone(),
            principal: principal.clone(),
            device: DeviceId("bot".into()),
            scope: BTreeSet::from(["navigate".into(), "stop".into()]),
            expires_at: MonotonicInstant(50),
            revoked: false,
        };
        let command = TypedCommand {
            command_id: "c".into(),
            operation,
            principal,
            sequence: CommandSequence(1),
            device: DeviceId("bot".into()),
            schema: "navigate".into(),
            payload: serde_json::json!({"x":1.0,"y":2.0}),
            deadline: MonotonicInstant(40),
        };
        (clock, robot, permit, command)
    }
    #[test]
    fn valid_command_moves_and_correlates_receipt() {
        let (_, mut r, p, c) = setup();
        let out = r.execute(&c, Some(&p));
        assert!(out.accepted());
        assert_eq!(r.position(), (1.0, 2.0));
        assert_eq!(out.operation, c.operation);
    }
    #[test]
    fn permit_lease_deadline_and_sequence_fail_closed() {
        let (clock, mut r, p, c) = setup();
        assert_eq!(
            r.execute(&c, None).decision,
            CommandDecision::Rejected(RejectionReason::MissingPermit)
        );
        let mut wrong = p.clone();
        wrong.principal = PrincipalId("mallory".into());
        assert_eq!(
            r.execute(&c, Some(&wrong)).decision,
            CommandDecision::Rejected(RejectionReason::PermitPrincipalMismatch)
        );
        assert!(r.execute(&c, Some(&p)).accepted());
        let position = r.position();
        assert_eq!(
            r.execute(&c, Some(&p)).decision,
            CommandDecision::Rejected(RejectionReason::ReplayOrOutOfOrder)
        );
        assert_eq!(r.position(), position);
        clock.advance_to(40).unwrap();
        let mut late = c.clone();
        late.sequence = CommandSequence(2);
        let receipt = r.execute(&late, Some(&p));
        assert_eq!(
            receipt.decision,
            CommandDecision::Rejected(RejectionReason::ExpiredDeadline)
        );
        assert_eq!(r.safety(), SafetyState::SafeStopped);
    }
    #[test]
    fn mismatched_authority_and_schema_are_exhaustively_rejected() {
        let (_, mut r, p, c) = setup();
        let mut value = p.clone();
        value.operation = OperationId("other".into());
        assert_eq!(
            r.execute(&c, Some(&value)).decision,
            CommandDecision::Rejected(RejectionReason::PermitOperationMismatch)
        );
        let mut value = p.clone();
        value.device = DeviceId("other".into());
        assert_eq!(
            r.execute(&c, Some(&value)).decision,
            CommandDecision::Rejected(RejectionReason::PermitDeviceMismatch)
        );
        let mut value = p.clone();
        value.scope.clear();
        assert_eq!(
            r.execute(&c, Some(&value)).decision,
            CommandDecision::Rejected(RejectionReason::PermitOutOfScope)
        );
        let mut value = c.clone();
        value.device = DeviceId("other".into());
        let mut permit = p.clone();
        permit.device = value.device.clone();
        assert_eq!(
            r.execute(&value, Some(&permit)).decision,
            CommandDecision::Rejected(RejectionReason::WrongDevice)
        );
        let mut value = c.clone();
        value.schema = "unknown".into();
        let mut permit = p.clone();
        permit.scope.insert("unknown".into());
        assert_eq!(
            r.execute(&value, Some(&permit)).decision,
            CommandDecision::Rejected(RejectionReason::SchemaMismatch)
        );
    }
    #[test]
    fn expiry_disconnect_fault_and_stop_are_safe() {
        let (clock, mut r, p, mut c) = setup();
        assert!(r.execute(&c, Some(&p)).accepted());
        clock.advance_to(50).unwrap();
        r.enforce_expiry();
        assert_eq!(r.safety(), SafetyState::SafeStopped);
        c.schema = "stop".into();
        c.sequence = CommandSequence(2);
        c.deadline = MonotonicInstant(60);
        let mut stop_permit = p.clone();
        stop_permit.expires_at = MonotonicInstant(60);
        assert!(matches!(
            r.execute(&c, Some(&stop_permit)).decision,
            CommandDecision::FailSafeApplied
        ));
        c.sequence = CommandSequence(3);
        assert!(matches!(
            r.execute(&c, Some(&stop_permit)).decision,
            CommandDecision::FailSafeApplied
        ));
        r.disconnect();
        assert_eq!(r.safety(), SafetyState::SafeStopped);
        r.fault();
        c.schema = "navigate".into();
        c.sequence = CommandSequence(4);
        assert_eq!(
            r.execute(&c, Some(&stop_permit)).decision,
            CommandDecision::Rejected(RejectionReason::UnsafeState)
        );
    }
    #[test]
    fn telemetry_and_clock_are_monotonic() {
        let (clock, mut r, _, _) = setup();
        let a = r.telemetry(None);
        clock.advance_by(1);
        let b = r.telemetry(Some(OperationId("op".into())));
        assert!(b.sequence > a.sequence);
        assert!(b.source_time > a.source_time);
        assert!(clock.advance_to(1).is_err());
    }
}

#[cfg(test)]
mod embodiment_tests {
    use super::*;
    use crate::{skill::authorized_fixture, EmbodimentProvider, ManualClock, SkillProgressSink};
    use fabric::{DeviceId, SkillId, SkillOutcome, SkillProgress, SkillRequest};

    struct NullSink;
    #[async_trait::async_trait]
    impl SkillProgressSink for NullSink {
        async fn progress(&self, _update: SkillProgress) {}
    }

    #[tokio::test]
    async fn simulated_provider_executes_only_with_matching_authority() {
        let clock = Arc::new(ManualClock::new(0));
        let provider = SimulatedEmbodiment::mobile_robot("bot", clock);
        let request = SkillRequest {
            skill: SkillId("navigate".into()),
            device: DeviceId("bot".into()),
            parameters: serde_json::json!({"x": 2.0, "y": 3.0}),
        };
        let authorized = authorized_fixture(request);
        let result = provider
            .execute_skill(
                crate::ValidatedSkillCommand(&authorized),
                Arc::new(NullSink),
            )
            .await
            .unwrap();
        assert_eq!(result.outcome, SkillOutcome::Succeeded);
    }

    #[tokio::test]
    async fn mismatched_permit_is_rejected_without_success() {
        let clock = Arc::new(ManualClock::new(0));
        let provider = SimulatedEmbodiment::mobile_robot("bot", clock);
        let request = SkillRequest {
            skill: SkillId("navigate".into()),
            device: DeviceId("bot".into()),
            parameters: serde_json::json!({"x": 2.0, "y": 3.0}),
        };
        let mut authorized = authorized_fixture(request);
        authorized.permit.operation = OperationId(fabric::OperationId::new().0.to_string());
        let result = provider
            .execute_skill(
                crate::ValidatedSkillCommand(&authorized),
                Arc::new(NullSink),
            )
            .await
            .unwrap();
        assert!(matches!(result.outcome, SkillOutcome::Failed { .. }));
    }

    #[tokio::test]
    async fn cancellation_applies_internal_safe_stop() {
        let clock = Arc::new(ManualClock::new(0));
        let provider = SimulatedEmbodiment::mobile_robot("bot", clock);
        let acknowledgement = provider
            .cancel(&DeviceId("bot".into()), &OperationId("op".into()))
            .await
            .unwrap();
        assert_eq!(acknowledgement.device, DeviceId("bot".into()));
        assert_eq!(
            provider.inner.lock().await.safety(),
            SafetyState::SafeStopped
        );
    }
}
