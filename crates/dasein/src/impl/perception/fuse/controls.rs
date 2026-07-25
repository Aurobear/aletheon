//! Write validation for the FUSE controls/ directory.
//!
//! The `ControlsValidator` enforces policy on writes to control files,
//! ensuring only valid values are accepted and security constraints are met.

use tracing::warn;

/// Result of a control write validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlVerdict {
    /// Write is allowed.
    Allow,
    /// Write is denied with a reason.
    Deny { reason: String },
}

/// Validates writes to control files in the FUSE filesystem.
///
/// Control files are paths under `/controls/` that accept commands like
/// pause/resume and configuration changes.
pub struct ControlsValidator {
    /// Allowed control file paths (without /controls/ prefix).
    allowed_controls: Vec<String>,
}

impl ControlsValidator {
    /// Create a validator with default allowed controls.
    pub fn with_defaults() -> Self {
        Self {
            allowed_controls: vec![
                "pause".to_string(),
                "resume".to_string(),
                "config.toml".to_string(),
            ],
        }
    }

    /// Create a validator with custom allowed controls.
    pub fn new(allowed_controls: Vec<String>) -> Self {
        Self { allowed_controls }
    }

    /// Validate a write to a control path.
    ///
    /// The `path` should be the full path (e.g., "/controls/pause").
    /// The `data` is the content being written.
    pub fn validate_write(&self, path: &str, data: &[u8]) -> ControlVerdict {
        // Extract the control name from the path
        let control_name = match path.strip_prefix("/controls/") {
            Some(name) => name,
            None => {
                return ControlVerdict::Deny {
                    reason: format!("Path is not under /controls/: {path}"),
                };
            }
        };

        // Check if the control is in the allowed list
        if !self.allowed_controls.contains(&control_name.to_string()) {
            warn!(path = path, "Write to unknown control denied");
            return ControlVerdict::Deny {
                reason: format!("Unknown control: {control_name}"),
            };
        }

        // Validate data for specific controls
        match control_name {
            "pause" | "resume" => self.validate_toggle(data),
            "config.toml" => self.validate_config(data),
            _ => ControlVerdict::Allow,
        }
    }

    /// Validate toggle controls (pause/resume) accept only "0" or "1".
    fn validate_toggle(&self, data: &[u8]) -> ControlVerdict {
        match std::str::from_utf8(data) {
            Ok(s) if s == "0" || s == "1" => ControlVerdict::Allow,
            Ok(s) => ControlVerdict::Deny {
                reason: format!("Toggle control must be '0' or '1', got: '{s}'"),
            },
            Err(_) => ControlVerdict::Deny {
                reason: "Toggle control data must be valid UTF-8".to_string(),
            },
        }
    }

    /// Validate config.toml writes (basic TOML syntax check).
    fn validate_config(&self, data: &[u8]) -> ControlVerdict {
        let content = match std::str::from_utf8(data) {
            Ok(s) => s,
            Err(_) => {
                return ControlVerdict::Deny {
                    reason: "Config data must be valid UTF-8".to_string(),
                };
            }
        };

        // Basic TOML validation — ensure it parses
        match content.parse::<toml::Value>() {
            Ok(_) => ControlVerdict::Allow,
            Err(e) => ControlVerdict::Deny {
                reason: format!("Invalid TOML: {e}"),
            },
        }
    }
}

impl Default for ControlsValidator {
    fn default() -> Self {
        Self::with_defaults()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_pause_valid() {
        let validator = ControlsValidator::with_defaults();
        assert_eq!(
            validator.validate_write("/controls/pause", b"0"),
            ControlVerdict::Allow
        );
        assert_eq!(
            validator.validate_write("/controls/pause", b"1"),
            ControlVerdict::Allow
        );
    }

    #[test]
    fn test_validate_pause_invalid_value() {
        let validator = ControlsValidator::with_defaults();
        let result = validator.validate_write("/controls/pause", b"2");
        assert!(matches!(result, ControlVerdict::Deny { .. }));
        if let ControlVerdict::Deny { reason } = result {
            assert!(reason.contains("'0' or '1'"));
        }
    }

    #[test]
    fn test_validate_resume_valid() {
        let validator = ControlsValidator::with_defaults();
        assert_eq!(
            validator.validate_write("/controls/resume", b"1"),
            ControlVerdict::Allow
        );
    }

    #[test]
    fn test_validate_non_utf8_toggle() {
        let validator = ControlsValidator::with_defaults();
        let result = validator.validate_write("/controls/pause", &[0xFF, 0xFE]);
        assert!(matches!(result, ControlVerdict::Deny { .. }));
    }

    #[test]
    fn test_validate_unknown_control() {
        let validator = ControlsValidator::with_defaults();
        let result = validator.validate_write("/controls/evil_command", b"1");
        assert!(matches!(result, ControlVerdict::Deny { .. }));
        if let ControlVerdict::Deny { reason } = result {
            assert!(reason.contains("Unknown control"));
        }
    }

    #[test]
    fn test_validate_not_controls_path() {
        let validator = ControlsValidator::with_defaults();
        let result = validator.validate_write("/sensors/cpu.json", b"{}");
        assert!(matches!(result, ControlVerdict::Deny { .. }));
    }

    #[test]
    fn test_validate_config_valid_toml() {
        let validator = ControlsValidator::with_defaults();
        let toml = b"[general]\nverbose = true\n";
        assert_eq!(
            validator.validate_write("/controls/config.toml", toml),
            ControlVerdict::Allow
        );
    }

    #[test]
    fn test_validate_config_invalid_toml() {
        let validator = ControlsValidator::with_defaults();
        let bad_toml = b"this is not valid toml [[[";
        let result = validator.validate_write("/controls/config.toml", bad_toml);
        assert!(matches!(result, ControlVerdict::Deny { .. }));
        if let ControlVerdict::Deny { reason } = result {
            assert!(reason.contains("Invalid TOML"));
        }
    }

    #[test]
    fn test_validate_config_non_utf8() {
        let validator = ControlsValidator::with_defaults();
        let result = validator.validate_write("/controls/config.toml", &[0xFF, 0xFE]);
        assert!(matches!(result, ControlVerdict::Deny { .. }));
    }

    #[test]
    fn test_custom_validator() {
        let validator = ControlsValidator::new(vec!["pause".to_string()]);
        // pause is allowed
        assert_eq!(
            validator.validate_write("/controls/pause", b"1"),
            ControlVerdict::Allow
        );
        // resume is NOT in the custom list
        let result = validator.validate_write("/controls/resume", b"1");
        assert!(matches!(result, ControlVerdict::Deny { .. }));
    }

    #[test]
    fn test_validate_toggle_whitespace_rejected() {
        let validator = ControlsValidator::with_defaults();
        let result = validator.validate_write("/controls/pause", b"1\n");
        assert!(matches!(result, ControlVerdict::Deny { .. }));
    }
}
