//! IntegrityMonitor: verify file integrity and detect tampering.
//!
//! Monitors binaries, configs, and security policy files for unauthorized changes:
//! 1. Computes and stores baseline hashes
//! 2. Periodically checks file integrity
//! 3. Triggers killswitch on violations
//! 4. Records violation history

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use fabric::MonoTime;
use serde::{Deserialize, Serialize};

use super::emergency_killswitch::EmergencyKillswitch;

/// Helper: compute elapsed Duration between two MonoTime values.
fn mono_elapsed(now: MonoTime, earlier: MonoTime) -> Duration {
    Duration::from_millis(now.0.saturating_sub(earlier.0))
}

/// Compute a deterministic hash of file contents.
///
/// Uses a simple FNV-1a variant for portability (no external deps).
/// Returns a 16-character hex string.
pub fn compute_file_hash(path: &PathBuf) -> Result<String> {
    let contents =
        std::fs::read(path).with_context(|| format!("Failed to read file: {}", path.display()))?;

    // FNV-1a parameters
    let mut hash: u64 = 0xcbf29ce484222325; // FNV offset basis
    let prime: u64 = 0x100000001b3; // FNV prime

    for byte in &contents {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(prime);
    }

    Ok(format!("{:016x}", hash))
}

/// What to check and how often.
#[derive(Debug, Clone)]
pub struct IntegrityCheck {
    /// Human-readable name for this check.
    pub name: String,
    /// What type of file to check.
    pub check_type: CheckType,
    /// How often to check.
    pub interval: Duration,
    /// When we last checked.
    pub last_checked: Option<MonoTime>,
    /// The hash we computed last time.
    pub last_hash: Option<String>,
}

/// Types of files to monitor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CheckType {
    /// SHA256 of executable binary.
    Binary { path: PathBuf },
    /// SHA256 of TOML config file.
    Config { path: PathBuf },
    /// SHA256 of security policy file.
    SecurityPolicy { path: PathBuf },
}

impl CheckType {
    /// Get the path being monitored.
    pub fn path(&self) -> &PathBuf {
        match self {
            CheckType::Binary { path } => path,
            CheckType::Config { path } => path,
            CheckType::SecurityPolicy { path } => path,
        }
    }
}

/// A record of an integrity violation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegrityViolation {
    /// Which check failed.
    pub check_name: String,
    /// What hash we expected.
    pub expected_hash: String,
    /// What hash we actually found.
    pub actual_hash: String,
    /// When the violation occurred (epoch millis).
    pub timestamp: u64,
}

/// Monitors file integrity and detects tampering.
pub struct IntegrityMonitor {
    /// All registered checks.
    checks: Vec<IntegrityCheck>,
    /// Expected hashes for each check (name → hash).
    baseline_hashes: HashMap<String, String>,
    /// History of violations.
    violations: Vec<IntegrityViolation>,
    /// Optional killswitch to trigger on violation.
    killswitch: Option<Arc<Mutex<EmergencyKillswitch>>>,
    clock: Arc<dyn fabric::Clock>,
}

impl IntegrityMonitor {
    /// Create a new IntegrityMonitor without killswitch integration.
    pub fn new(clock: Arc<dyn fabric::Clock>) -> Self {
        Self {
            checks: Vec::new(),
            baseline_hashes: HashMap::new(),
            violations: Vec::new(),
            killswitch: None,
            clock,
        }
    }

    /// Create a new IntegrityMonitor with killswitch integration.
    pub fn with_killswitch(killswitch: Arc<Mutex<EmergencyKillswitch>>, clock: Arc<dyn fabric::Clock>) -> Self {
        Self {
            checks: Vec::new(),
            baseline_hashes: HashMap::new(),
            violations: Vec::new(),
            killswitch: Some(killswitch),
            clock,
        }
    }

    /// Register a new integrity check.
    pub fn register_check(&mut self, check: IntegrityCheck) {
        self.checks.push(check);
    }

    /// Set the baseline hash for a check.
    pub fn set_baseline(&mut self, name: &str, hash: String) {
        self.baseline_hashes.insert(name.to_string(), hash);
    }

    /// Run all checks whose interval has elapsed.
    ///
    /// Returns the number of violations detected.
    pub fn check_all(&mut self) -> Result<usize> {
        let now = self.clock.mono_now();
        let mut violations = 0;

        // Collect checks that need to run
        let mut checks_to_run = Vec::new();
        for (idx, check) in self.checks.iter().enumerate() {
            if let Some(last_checked) = check.last_checked {
                if mono_elapsed(now, last_checked) < check.interval {
                    continue;
                }
            }
            checks_to_run.push(idx);
        }

        // Run the checks
        for idx in checks_to_run {
            if self.run_check(idx, now)? {
                violations += 1;
            }
            self.checks[idx].last_checked = Some(now);
        }

        Ok(violations)
    }

    /// Run a specific check by name.
    ///
    /// Returns true if a violation was detected.
    pub fn check_one(&mut self, name: &str) -> Result<bool> {
        let idx = self.checks.iter().position(|c| c.name == name);

        match idx {
            Some(idx) => {
                let now = self.clock.mono_now();
                let result = self.run_check(idx, now)?;
                self.checks[idx].last_checked = Some(now);
                Ok(result)
            }
            None => Err(anyhow::anyhow!("Check not found: {}", name)),
        }
    }

    /// Run a single check by index and record violations.
    ///
    /// Returns true if a violation was detected.
    fn run_check(&mut self, idx: usize, _now: MonoTime) -> Result<bool> {
        let path = self.checks[idx].check_type.path().clone();
        let name = self.checks[idx].name.clone();
        let actual_hash = compute_file_hash(&path)?;

        // Update last_hash
        self.checks[idx].last_hash = Some(actual_hash.clone());

        // Check against baseline
        if let Some(expected_hash) = self.baseline_hashes.get(&name) {
            if *actual_hash != *expected_hash {
                // Violation detected!
                let violation = IntegrityViolation {
                    check_name: name.clone(),
                    expected_hash: expected_hash.clone(),
                    actual_hash: actual_hash.clone(),
                    timestamp: self.clock.wall_now().0 as u64,
                };

                tracing::error!(
                    check = %name,
                    expected = %violation.expected_hash,
                    actual = %violation.actual_hash,
                    "Integrity violation detected"
                );

                // Trigger killswitch if attached
                if let Some(killswitch) = &self.killswitch {
                    let ks = killswitch.lock().unwrap();
                    ks.report_violation(&format!(
                        "Integrity violation: {} (expected {}, got {})",
                        name, violation.expected_hash, violation.actual_hash
                    ));
                }

                self.violations.push(violation);
                return Ok(true);
            }
        }

        Ok(false)
    }

    /// Returns all recorded violations.
    pub fn violations(&self) -> &[IntegrityViolation] {
        &self.violations
    }

    /// Clear the violation history.
    pub fn clear_violations(&mut self) {
        self.violations.clear();
    }
}
