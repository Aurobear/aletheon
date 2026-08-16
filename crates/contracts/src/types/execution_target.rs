//! Explicit, per-turn execution target selected by a trusted presentation edge.
//!
//! Natural-language prompt content is deliberately absent from this contract:
//! model text must never upgrade a General turn into an embodied Robot turn.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::embodiment::{DeviceId, ExecutionEnvironment};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExecutionTarget {
    #[default]
    General,
    Robot {
        device_id: DeviceId,
        environment: ExecutionEnvironment,
    },
}

impl ExecutionTarget {
    pub fn label(&self) -> String {
        match self {
            Self::General => "general".into(),
            Self::Robot { device_id, .. } => format!("robot:{}", device_id.0),
        }
    }
}

/// Auditable provenance for the typed target selection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionTargetSource {
    /// Backward-compatible/default path. This source may select only General.
    #[default]
    Default,
    /// Explicit local UI command such as `/target robot <device>`.
    UserCommand,
    /// Explicit typed field supplied by an authenticated trusted client.
    TrustedClient,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ExecutionTargetSelection {
    #[serde(default)]
    pub target: ExecutionTarget,
    #[serde(default)]
    pub source: ExecutionTargetSource,
}

impl ExecutionTargetSelection {
    pub fn general(source: ExecutionTargetSource) -> Self {
        Self {
            target: ExecutionTarget::General,
            source,
        }
    }

    pub fn robot(
        device_id: impl Into<String>,
        environment: ExecutionEnvironment,
        source: ExecutionTargetSource,
    ) -> Result<Self, String> {
        let selection = Self {
            target: ExecutionTarget::Robot {
                device_id: DeviceId(device_id.into()),
                environment,
            },
            source,
        };
        selection.validate()?;
        Ok(selection)
    }

    pub fn validate(&self) -> Result<(), String> {
        if let ExecutionTarget::Robot { device_id, .. } = &self.target {
            if device_id.0.trim().is_empty() {
                return Err("robot execution target requires a non-empty device_id".into());
            }
            if self.source == ExecutionTargetSource::Default {
                return Err(
                    "robot execution target requires an explicit user command or trusted client field"
                        .into(),
                );
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_general_and_auditable() {
        let selection = ExecutionTargetSelection::default();
        assert_eq!(selection.target, ExecutionTarget::General);
        assert_eq!(selection.source, ExecutionTargetSource::Default);
        selection.validate().unwrap();
    }

    #[test]
    fn robot_cannot_use_implicit_source() {
        let selection = ExecutionTargetSelection {
            target: ExecutionTarget::Robot {
                device_id: DeviceId("robot-1".into()),
                environment: ExecutionEnvironment::Simulation,
            },
            source: ExecutionTargetSource::Default,
        };
        assert!(selection.validate().is_err());
    }
}
