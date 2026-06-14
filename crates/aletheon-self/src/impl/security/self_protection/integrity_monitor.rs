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
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::emergency_killswitch::EmergencyKillswitch;

/// Get current epoch millis (for timestamp tracking).
fn epoch_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Compute a deterministic hash of file contents.
///
/// Uses a simple FNV-1a variant for portability (no external deps).
/// Returns a 16-character hex string.
pub fn compute_file_hash(path: &PathBuf) -> Result<String> {
    let contents = std::fs::read(path)
        .with_context(|| format!("Failed to read file: {}", path.display()))?;

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
    pub last_checked: Option<Instant>,
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
}

impl IntegrityMonitor {
    /// Create a new IntegrityMonitor without killswitch integration.
    pub fn new() -> Self {
        Self {
            checks: Vec::new(),
            baseline_hashes: HashMap::new(),
            violations: Vec::new(),
            killswitch: None,
        }
    }

    /// Create a new IntegrityMonitor with killswitch integration.
    pub fn with_killswitch(killswitch: Arc<Mutex<EmergencyKillswitch>>) -> Self {
        Self {
            checks: Vec::new(),
            baseline_hashes: HashMap::new(),
            violations: Vec::new(),
            killswitch: Some(killswitch),
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
        let now = Instant::now();
        let mut violations = 0;

        // Collect checks that need to run
        let mut checks_to_run = Vec::new();
        for (idx, check) in self.checks.iter().enumerate() {
            if let Some(last_checked) = check.last_checked {
                if now.duration_since(last_checked) < check.interval {
                    continue;
                }
            }
            checks_to_run.push(idx);
        }

        // Run the checks
        for idx in checks_to_run {
            if self.run_check(idx)? {
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
                let result = self.run_check(idx)?;
                self.checks[idx].last_checked = Some(Instant::now());
                Ok(result)
            }
            None => Err(anyhow::anyhow!("Check not found: {}", name)),
        }
    }

    /// Run a single check by index and record violations.
    ///
    /// Returns true if a violation was detected.
    fn run_check(&mut self, idx: usize) -> Result<bool> {
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
                    timestamp: epoch_millis(),
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

    /// Get all violations.
    pub fn violations(&self) -> &[IntegrityViolation] {
        &self.violations
    }

    /// Clear violation history.
    pub fn clear_violations(&mut self) {
        self.violations.clear();
    }

    /// Get the number of registered checks.
    pub fn check_count(&self) -> usize {
        self.checks.len()
    }

    /// Get a check by name.
    pub fn get_check(&self, name: &str) -> Option<&IntegrityCheck> {
        self.checks.iter().find(|c| c.name == name)
    }

    /// Get the baseline hash for a check.
    pub fn get_baseline(&self, name: &str) -> Option<&String> {
        self.baseline_hashes.get(name)
    }
}

impl Default for IntegrityMonitor {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn test_compute_file_hash_consistency() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        let mut file = File::create(&file_path).unwrap();
        writeln!(file, "test content").unwrap();

        let hash1 = compute_file_hash(&file_path).unwrap();
        let hash2 = compute_file_hash(&file_path).unwrap();

        assert_eq!(hash1, hash2);
        assert_eq!(hash1.len(), 16);
    }

    #[test]
    fn test_compute_file_hash_different_content() {
        let dir = tempdir().unwrap();
        let file1 = dir.path().join("file1.txt");
        let file2 = dir.path().join("file2.txt");

        let mut f1 = File::create(&file1).unwrap();
        writeln!(f1, "content A").unwrap();

        let mut f2 = File::create(&file2).unwrap();
        writeln!(f2, "content B").unwrap();

        let hash1 = compute_file_hash(&file1).unwrap();
        let hash2 = compute_file_hash(&file2).unwrap();

        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_set_baseline_and_check_no_violation() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("config.toml");
        let mut file = File::create(&file_path).unwrap();
        writeln!(file, "key = 'value'").unwrap();

        let hash = compute_file_hash(&file_path).unwrap();

        let mut monitor = IntegrityMonitor::new();
        monitor.register_check(IntegrityCheck {
            name: "test-config".to_string(),
            check_type: CheckType::Config {
                path: file_path.clone(),
            },
            interval: Duration::from_secs(0),
            last_checked: None,
            last_hash: None,
        });
        monitor.set_baseline("test-config", hash);

        let violations = monitor.check_all().unwrap();
        assert_eq!(violations, 0);
        assert!(monitor.violations().is_empty());
    }

    #[test]
    fn test_violation_detection_on_tamper() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("config.toml");

        // Write initial content
        {
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "key = 'original'").unwrap();
        }

        let original_hash = compute_file_hash(&file_path).unwrap();

        let mut monitor = IntegrityMonitor::new();
        monitor.register_check(IntegrityCheck {
            name: "test-config".to_string(),
            check_type: CheckType::Config {
                path: file_path.clone(),
            },
            interval: Duration::from_secs(0),
            last_checked: None,
            last_hash: None,
        });
        monitor.set_baseline("test-config", original_hash);

        // Tamper with the file
        {
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "key = 'tampered'").unwrap();
        }

        let violations = monitor.check_all().unwrap();
        assert_eq!(violations, 1);
        assert_eq!(monitor.violations().len(), 1);
        assert_eq!(monitor.violations()[0].check_name, "test-config");
    }

    #[test]
    fn test_killswitch_trigger_on_violation() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("security.policy");

        // Write initial content
        {
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "allow all").unwrap();
        }

        let original_hash = compute_file_hash(&file_path).unwrap();

        let killswitch = Arc::new(Mutex::new(EmergencyKillswitch::new()));
        let mut monitor = IntegrityMonitor::with_killswitch(killswitch.clone());

        monitor.register_check(IntegrityCheck {
            name: "security-policy".to_string(),
            check_type: CheckType::SecurityPolicy {
                path: file_path.clone(),
            },
            interval: Duration::from_secs(0),
            last_checked: None,
            last_hash: None,
        });
        monitor.set_baseline("security-policy", original_hash);

        // Verify killswitch is not active initially
        assert!(!killswitch.lock().unwrap().is_active());

        // Tamper with the file
        {
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "allow none").unwrap();
        }

        let violations = monitor.check_all().unwrap();
        assert_eq!(violations, 1);

        // Verify killswitch was triggered
        assert!(killswitch.lock().unwrap().is_active());
    }

    #[test]
    fn test_check_interval_enforcement() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("binary");

        // Write initial content
        {
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "binary content").unwrap();
        }

        let original_hash = compute_file_hash(&file_path).unwrap();

        let mut monitor = IntegrityMonitor::new();
        monitor.register_check(IntegrityCheck {
            name: "test-binary".to_string(),
            check_type: CheckType::Binary {
                path: file_path.clone(),
            },
            interval: Duration::from_secs(60), // 60 second interval
            last_checked: None,
            last_hash: None,
        });
        monitor.set_baseline("test-binary", original_hash);

        // First check should run
        let violations = monitor.check_all().unwrap();
        assert_eq!(violations, 0);

        // Tamper with the file
        {
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "tampered binary").unwrap();
        }

        // Second check should NOT run (interval not elapsed)
        let violations = monitor.check_all().unwrap();
        assert_eq!(violations, 0);
        assert!(monitor.violations().is_empty());
    }

    #[test]
    fn test_check_one_specific_check() {
        let dir = tempdir().unwrap();
        let file1 = dir.path().join("config1.toml");
        let file2 = dir.path().join("config2.toml");

        // Write initial content
        {
            let mut f1 = File::create(&file1).unwrap();
            writeln!(f1, "key1 = 'value1'").unwrap();

            let mut f2 = File::create(&file2).unwrap();
            writeln!(f2, "key2 = 'value2'").unwrap();
        }

        let hash1 = compute_file_hash(&file1).unwrap();
        let hash2 = compute_file_hash(&file2).unwrap();

        let mut monitor = IntegrityMonitor::new();

        monitor.register_check(IntegrityCheck {
            name: "config1".to_string(),
            check_type: CheckType::Config {
                path: file1.clone(),
            },
            interval: Duration::from_secs(0),
            last_checked: None,
            last_hash: None,
        });

        monitor.register_check(IntegrityCheck {
            name: "config2".to_string(),
            check_type: CheckType::Config {
                path: file2.clone(),
            },
            interval: Duration::from_secs(0),
            last_checked: None,
            last_hash: None,
        });

        monitor.set_baseline("config1", hash1);
        monitor.set_baseline("config2", hash2);

        // Tamper with config2
        {
            let mut f2 = File::create(&file2).unwrap();
            writeln!(f2, "key2 = 'tampered'").unwrap();
        }

        // Check only config1 - should pass
        let has_violation = monitor.check_one("config1").unwrap();
        assert!(!has_violation);
        assert!(monitor.violations().is_empty());

        // Check config2 - should fail
        let has_violation = monitor.check_one("config2").unwrap();
        assert!(has_violation);
        assert_eq!(monitor.violations().len(), 1);
        assert_eq!(monitor.violations()[0].check_name, "config2");
    }

    #[test]
    fn test_multiple_check_types() {
        let dir = tempdir().unwrap();
        let binary_path = dir.path().join("agent");
        let config_path = dir.path().join("config.toml");
        let policy_path = dir.path().join("security.policy");

        // Write initial content
        {
            let mut f1 = File::create(&binary_path).unwrap();
            writeln!(f1, "ELF binary content").unwrap();

            let mut f2 = File::create(&config_path).unwrap();
            writeln!(f2, "[settings]\nkey = 'value'").unwrap();

            let mut f3 = File::create(&policy_path).unwrap();
            writeln!(f3, "allow read\ndeny write").unwrap();
        }

        let binary_hash = compute_file_hash(&binary_path).unwrap();
        let config_hash = compute_file_hash(&config_path).unwrap();
        let policy_hash = compute_file_hash(&policy_path).unwrap();

        let mut monitor = IntegrityMonitor::new();

        monitor.register_check(IntegrityCheck {
            name: "agent-binary".to_string(),
            check_type: CheckType::Binary {
                path: binary_path.clone(),
            },
            interval: Duration::from_secs(300),
            last_checked: None,
            last_hash: None,
        });

        monitor.register_check(IntegrityCheck {
            name: "main-config".to_string(),
            check_type: CheckType::Config {
                path: config_path.clone(),
            },
            interval: Duration::from_secs(60),
            last_checked: None,
            last_hash: None,
        });

        monitor.register_check(IntegrityCheck {
            name: "security-policy".to_string(),
            check_type: CheckType::SecurityPolicy {
                path: policy_path.clone(),
            },
            interval: Duration::from_secs(30),
            last_checked: None,
            last_hash: None,
        });

        monitor.set_baseline("agent-binary", binary_hash);
        monitor.set_baseline("main-config", config_hash);
        monitor.set_baseline("security-policy", policy_hash);

        // All checks should pass
        let violations = monitor.check_all().unwrap();
        assert_eq!(violations, 0);
        assert_eq!(monitor.check_count(), 3);
    }

    #[test]
    fn test_clear_violations() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.txt");

        // Write initial content
        {
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "original").unwrap();
        }

        let original_hash = compute_file_hash(&file_path).unwrap();

        let mut monitor = IntegrityMonitor::new();
        monitor.register_check(IntegrityCheck {
            name: "test".to_string(),
            check_type: CheckType::Config {
                path: file_path.clone(),
            },
            interval: Duration::from_secs(0),
            last_checked: None,
            last_hash: None,
        });
        monitor.set_baseline("test", original_hash);

        // Tamper and check
        {
            let mut file = File::create(&file_path).unwrap();
            writeln!(file, "tampered").unwrap();
        }

        monitor.check_all().unwrap();
        assert_eq!(monitor.violations().len(), 1);

        // Clear violations
        monitor.clear_violations();
        assert!(monitor.violations().is_empty());
    }

    #[test]
    fn test_get_check_and_baseline() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("config.toml");
        let mut file = File::create(&file_path).unwrap();
        writeln!(file, "test").unwrap();

        let hash = compute_file_hash(&file_path).unwrap();

        let mut monitor = IntegrityMonitor::new();
        monitor.register_check(IntegrityCheck {
            name: "test-config".to_string(),
            check_type: CheckType::Config {
                path: file_path.clone(),
            },
            interval: Duration::from_secs(60),
            last_checked: None,
            last_hash: None,
        });
        monitor.set_baseline("test-config", hash.clone());

        let check = monitor.get_check("test-config").unwrap();
        assert_eq!(check.name, "test-config");

        let baseline = monitor.get_baseline("test-config").unwrap();
        assert_eq!(baseline, &hash);

        assert!(monitor.get_check("nonexistent").is_none());
        assert!(monitor.get_baseline("nonexistent").is_none());
    }
}
