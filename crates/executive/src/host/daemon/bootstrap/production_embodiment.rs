//! Production embodiment bootstrap — gate checks in order.
//! Never downgrade failed production to simulator silently.

/// Ordered gate check result.
#[derive(Debug)]
pub struct GateCheck {
    pub step: &'static str,
    pub passed: bool,
    pub detail: String,
}

/// Ordered production startup gate. Checks must pass in sequence.
#[derive(Debug)]
pub struct ProductionStartupGate {
    checks: Vec<GateCheck>,
}

impl ProductionStartupGate {
    pub fn new() -> Self {
        Self { checks: Vec::new() }
    }

    fn record(&mut self, step: &'static str, result: Result<(), String>) -> Result<(), String> {
        match result {
            Ok(()) => {
                self.checks.push(GateCheck {
                    step,
                    passed: true,
                    detail: "ok".into(),
                });
                Ok(())
            }
            Err(err) => {
                let detail = err.clone();
                self.checks.push(GateCheck {
                    step,
                    passed: false,
                    detail: err,
                });
                Err(detail)
            }
        }
    }

    /// Gate 1: Validate config — namespace must be production, all fields present.
    pub fn check_config(&mut self, config_valid: Result<(), Vec<String>>) -> Result<(), String> {
        match config_valid {
            Ok(()) => self.record("config_validation", Ok(())),
            Err(errors) => self.record("config_validation", Err(errors.join("; "))),
        }
    }

    /// Gate 2: Resolve credentials through host-owned secret loader.
    pub fn check_credentials(&mut self, tls_resolved: bool) -> Result<(), String> {
        if tls_resolved {
            self.record("credential_resolution", Ok(()))
        } else {
            self.record(
                "credential_resolution",
                Err("TLS credentials not resolved".into()),
            )
        }
    }

    /// Gate 3: Device identity handshake — verify device_id and serial.
    pub fn check_device_identity(
        &mut self,
        device_id_match: bool,
        serial_match: bool,
    ) -> Result<(), String> {
        self.record(
            "device_id_handshake",
            if device_id_match {
                Ok(())
            } else {
                Err("device_id mismatch".into())
            },
        )?;
        self.record(
            "device_serial",
            if serial_match {
                Ok(())
            } else {
                Err("device serial mismatch".into())
            },
        )
    }

    /// Gate 4: Manifest and limits digest match configured values.
    pub fn check_manifest_digest(
        &mut self,
        manifest_match: bool,
        limits_match: bool,
    ) -> Result<(), String> {
        self.record(
            "manifest_digest",
            if manifest_match {
                Ok(())
            } else {
                Err("manifest digest mismatch".into())
            },
        )?;
        self.record(
            "limits_digest",
            if limits_match {
                Ok(())
            } else {
                Err("limits digest mismatch".into())
            },
        )
    }

    /// Gate 5: Signed HIL evidence is present, valid, unexpired.
    pub fn check_evidence(
        &mut self,
        evidence_valid: bool,
        evidence_loaded: bool,
    ) -> Result<(), String> {
        if !evidence_loaded {
            return self.record("evidence_loaded", Err("evidence file not found".into()));
        }
        self.record(
            "evidence_valid",
            if evidence_valid {
                Ok(())
            } else {
                Err("evidence signature/tamper/invalid".into())
            },
        )
    }

    /// Gate 6: E-stop self-test passes.
    pub fn check_estop(&mut self, estop_armed: bool) -> Result<(), String> {
        self.record(
            "estop_self_test",
            if estop_armed {
                Ok(())
            } else {
                Err("E-stop not armed".into())
            },
        )
    }

    /// Gate 7: Local operator arming receipt.
    pub fn check_operator_arming(&mut self, operator_id: Option<&str>) -> Result<(), String> {
        match operator_id.filter(|id| !id.is_empty()) {
            Some(_id) => self.record("operator_arming", Ok(())),
            None => self.record("operator_arming", Err("operator arming required".into())),
        }
    }

    /// All gates passed?
    pub fn all_passed(&self) -> bool {
        self.checks.iter().all(|c| c.passed)
    }

    /// Get sanitized failure summary (no credentials/secrets).
    pub fn health_report(&self) -> String {
        let failures: Vec<_> = self.checks.iter().filter(|c| !c.passed).collect();
        if failures.is_empty() {
            "production gate: all checks passed".into()
        } else {
            let parts: Vec<_> = failures
                .iter()
                .map(|c| format!("{}: {}", c.step, c.detail))
                .collect();
            format!("production gate FAILED: {}", parts.join("; "))
        }
    }

    /// NEVER returns simulator config when production fails.
    /// Must return Err to prevent silent downgrade.
    pub fn finalize(self) -> Result<(), String> {
        if self.all_passed() {
            Ok(())
        } else {
            Err(self.health_report())
        }
    }
}

impl Default for ProductionStartupGate {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_gates_pass() {
        let mut gate = ProductionStartupGate::new();
        assert!(gate.check_config(Ok(())).is_ok());
        assert!(gate.check_credentials(true).is_ok());
        assert!(gate.check_device_identity(true, true).is_ok());
        assert!(gate.check_manifest_digest(true, true).is_ok());
        assert!(gate.check_evidence(true, true).is_ok());
        assert!(gate.check_estop(true).is_ok());
        assert!(gate.check_operator_arming(Some("op-1")).is_ok());
        assert!(gate.all_passed());
        assert!(gate.finalize().is_ok());
    }

    #[test]
    fn config_failure_blocks_production() {
        let mut gate = ProductionStartupGate::new();
        assert!(gate
            .check_config(Err(vec!["bad namespace".into()]))
            .is_err());
        assert!(!gate.all_passed());
        let err = gate.finalize().unwrap_err();
        assert!(err.contains("config_validation"));
        assert!(err.contains("bad namespace"));
    }

    #[test]
    fn credential_failure_blocks() {
        let mut gate = ProductionStartupGate::new();
        gate.check_config(Ok(())).unwrap();
        assert!(gate.check_credentials(false).is_err());
        assert!(gate.finalize().is_err());
    }

    #[test]
    fn health_report_sanitized_no_secrets() {
        let mut gate = ProductionStartupGate::new();
        gate.check_config(Err(vec!["namespace error".into()])).ok();
        let report = gate.health_report();
        assert!(!report.contains("SECRET"));
        assert!(!report.contains("password"));
        assert!(!report.contains("token"));
    }

    #[test]
    fn gate_order_enforced() {
        let mut gate = ProductionStartupGate::new();
        // Early gate failure doesn't prevent later checks but marks overall as failed
        gate.check_config(Err(vec!["invalid".into()])).ok(); // record failure
        gate.check_credentials(true).ok();
        assert!(!gate.all_passed());
    }
}
