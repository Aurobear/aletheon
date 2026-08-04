//! Stable command and intent values shared by presentation adapters.
//!
//! Keeping these contracts free of Clap prevents neutral Fabric types from
//! depending on a presentation framework. Adapters classify syntax into a
//! [`ClientIntent`]; only the Executive application layer selects a use case.

use std::fmt;
use std::str::FromStr;
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::types::admission::PrincipalId;
use crate::types::evaluation::TaskKind;
use crate::types::local_authority::WorkspacePolicy;
use crate::types::permission::HostPermissionMode;
use crate::types::space::SessionId;
use crate::TurnRequirement;

pub const CLIENT_INTENT_SCHEMA_V1: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientSurface {
    Cli,
    Tui,
    Gateway,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandId {
    SubmitPrompt,
    Status,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CommandSurface {
    Cli,
    Tui,
    Gateway,
}

impl CommandSurface {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "cli" => Some(Self::Cli),
            "tui" => Some(Self::Tui),
            "gateway" => Some(Self::Gateway),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandExecution {
    Local,
    Application,
    Compatibility,
}

impl CommandExecution {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "local" => Some(Self::Local),
            "application" => Some(Self::Application),
            "compatibility" => Some(Self::Compatibility),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandAvailability {
    Always,
    IdleOnly,
    ActiveTurnOnly,
}

impl CommandAvailability {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "always" => Some(Self::Always),
            "idle_only" => Some(Self::IdleOnly),
            "active_turn_only" => Some(Self::ActiveTurnOnly),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandVisibility {
    Public,
    Internal,
    Compatibility,
}

impl CommandVisibility {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "public" => Some(Self::Public),
            "internal" => Some(Self::Internal),
            "compatibility" => Some(Self::Compatibility),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    pub key: String,
    pub surface: CommandSurface,
    pub parent: Option<String>,
    pub name: String,
    pub aliases: Vec<String>,
    pub summary: String,
    pub category: String,
    pub usage: String,
    pub execution: CommandExecution,
    pub availability: CommandAvailability,
    pub visibility: CommandVisibility,
    pub intent: Option<CommandId>,
}

pub static COMMAND_SPECS: LazyLock<Vec<CommandSpec>> =
    LazyLock::new(|| parse_command_specs(include_str!("command-specs.tsv")));

pub fn command_specs(surface: CommandSurface) -> impl Iterator<Item = &'static CommandSpec> {
    COMMAND_SPECS
        .iter()
        .filter(move |spec| spec.surface == surface)
}

pub fn resolve_command(
    surface: CommandSurface,
    parent: Option<&str>,
    name: &str,
) -> Option<&'static CommandSpec> {
    command_specs(surface).find(|spec| {
        spec.parent.as_deref() == parent
            && (spec.name == name || spec.aliases.iter().any(|alias| alias == name))
    })
}

pub fn command_spec(id: CommandId) -> &'static CommandSpec {
    COMMAND_SPECS
        .iter()
        .find(|spec| spec.intent == Some(id))
        .expect("every CommandId must have one CommandSpec")
}

fn parse_command_specs(input: &str) -> Vec<CommandSpec> {
    let mut specs = Vec::new();
    for (index, line) in input.lines().enumerate() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let columns = line.split('\t').collect::<Vec<_>>();
        assert_eq!(
            columns.len(),
            12,
            "invalid command spec column count at line {}",
            index + 1
        );
        let intent = match columns[11] {
            "-" => None,
            "submit_prompt" => Some(CommandId::SubmitPrompt),
            "status" => Some(CommandId::Status),
            other => panic!("invalid intent {other} at line {}", index + 1),
        };
        specs.push(CommandSpec {
            key: columns[0].to_owned(),
            surface: CommandSurface::parse(columns[1])
                .unwrap_or_else(|| panic!("invalid surface at line {}", index + 1)),
            parent: (columns[2] != "-").then(|| columns[2].to_owned()),
            name: columns[3].to_owned(),
            aliases: if columns[4] == "-" {
                Vec::new()
            } else {
                columns[4].split(',').map(str::to_owned).collect()
            },
            summary: columns[5].to_owned(),
            category: columns[6].to_owned(),
            usage: columns[7].to_owned(),
            execution: CommandExecution::parse(columns[8])
                .unwrap_or_else(|| panic!("invalid execution at line {}", index + 1)),
            availability: CommandAvailability::parse(columns[9])
                .unwrap_or_else(|| panic!("invalid availability at line {}", index + 1)),
            visibility: CommandVisibility::parse(columns[10])
                .unwrap_or_else(|| panic!("invalid visibility at line {}", index + 1)),
            intent,
        });
    }
    specs
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmitPromptIntent {
    pub content: String,
    pub session_id: Option<SessionId>,
    pub workspace: WorkspacePolicy,
    pub requirements: Vec<TurnRequirement>,
    pub task_kind: Option<TaskKind>,
    pub permission_mode: HostPermissionMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", content = "arguments", rename_all = "snake_case")]
pub enum ClientCommand {
    SubmitPrompt(SubmitPromptIntent),
    Status,
}

impl ClientCommand {
    pub const fn id(&self) -> CommandId {
        match self {
            Self::SubmitPrompt(_) => CommandId::SubmitPrompt,
            Self::Status => CommandId::Status,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientIntent {
    pub schema_version: u16,
    pub surface: ClientSurface,
    pub principal: PrincipalId,
    pub correlation_id: String,
    pub command: ClientCommand,
}

impl ClientIntent {
    pub fn v1(
        surface: ClientSurface,
        principal: PrincipalId,
        correlation_id: impl Into<String>,
        command: ClientCommand,
    ) -> Self {
        Self {
            schema_version: CLIENT_INTENT_SCHEMA_V1,
            surface,
            principal,
            correlation_id: correlation_id.into(),
            command,
        }
    }

    pub fn validate(&self) -> Result<(), ClientIntentError> {
        if self.schema_version != CLIENT_INTENT_SCHEMA_V1 {
            return Err(ClientIntentError::UnsupportedSchema(self.schema_version));
        }
        if self.principal.0.trim().is_empty() {
            return Err(ClientIntentError::EmptyPrincipal);
        }
        if self.correlation_id.trim().is_empty() {
            return Err(ClientIntentError::EmptyCorrelationId);
        }
        if let ClientCommand::SubmitPrompt(prompt) = &self.command {
            if prompt.content.trim().is_empty() {
                return Err(ClientIntentError::EmptyPrompt);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ClientIntentError {
    #[error("unsupported client intent schema {0}")]
    UnsupportedSchema(u16),
    #[error("client intent principal is empty")]
    EmptyPrincipal,
    #[error("client intent correlation id is empty")]
    EmptyCorrelationId,
    #[error("client intent prompt is empty")]
    EmptyPrompt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskKindArg {
    Coding,
}

impl FromStr for TaskKindArg {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "coding" => Ok(Self::Coding),
            other => Err(format!("unsupported task kind: {other}")),
        }
    }
}

impl fmt::Display for TaskKindArg {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Coding => formatter.write_str("coding"),
        }
    }
}

impl From<TaskKindArg> for TaskKind {
    fn from(value: TaskKindArg) -> Self {
        match value {
            TaskKindArg::Coding => Self::Coding,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_kind_argument_round_trips_without_presentation_dependencies() {
        let value = "coding".parse::<TaskKindArg>().unwrap();
        assert_eq!(value, TaskKindArg::Coding);
        assert_eq!(value.to_string(), "coding");
        assert_eq!(TaskKind::from(value), TaskKind::Coding);
    }

    #[test]
    fn task_kind_argument_rejects_unknown_values() {
        assert!("general".parse::<TaskKindArg>().is_err());
    }

    #[test]
    fn command_specs_cover_each_command_once() {
        assert_eq!(command_spec(CommandId::SubmitPrompt).name, "chat");
        assert_eq!(command_spec(CommandId::Status).aliases, ["st"]);

        let mut keys = std::collections::BTreeSet::new();
        let mut spellings = std::collections::BTreeSet::new();
        for spec in COMMAND_SPECS.iter() {
            assert!(keys.insert(spec.key.as_str()), "duplicate key {}", spec.key);
            let scope = (spec.surface, spec.parent.as_deref());
            assert!(
                spellings.insert((scope, spec.name.as_str())),
                "duplicate command spelling {}",
                spec.name
            );
            for alias in &spec.aliases {
                assert!(
                    spellings.insert((scope, alias.as_str())),
                    "duplicate command alias {alias}"
                );
            }
        }
        for id in [CommandId::SubmitPrompt, CommandId::Status] {
            assert_eq!(
                COMMAND_SPECS
                    .iter()
                    .filter(|spec| spec.intent == Some(id))
                    .count(),
                1
            );
        }
    }

    #[test]
    fn command_resolution_is_surface_and_parent_scoped() {
        assert_eq!(
            resolve_command(CommandSurface::Gateway, None, "chat").map(|spec| spec.key.as_str()),
            Some("gateway.chat")
        );
        assert_eq!(
            resolve_command(CommandSurface::Tui, None, "st").map(|spec| spec.key.as_str()),
            Some("tui.status")
        );
        assert_eq!(
            resolve_command(CommandSurface::Cli, Some("memory.workspace"), "bind")
                .map(|spec| spec.key.as_str()),
            Some("cli.memory.workspace.bind")
        );
        assert!(resolve_command(CommandSurface::Gateway, None, "st").is_none());
    }

    #[test]
    fn client_intent_validation_is_surface_neutral() {
        let workspace =
            WorkspacePolicy::from_resolved_roots("/tmp/project".into(), vec![]).unwrap();
        for surface in [
            ClientSurface::Cli,
            ClientSurface::Tui,
            ClientSurface::Gateway,
        ] {
            ClientIntent::v1(
                surface,
                PrincipalId("owner".into()),
                "correlation",
                ClientCommand::SubmitPrompt(SubmitPromptIntent {
                    content: "hello".into(),
                    session_id: None,
                    workspace: workspace.clone(),
                    requirements: vec![],
                    task_kind: None,
                    permission_mode: HostPermissionMode::Safe,
                }),
            )
            .validate()
            .unwrap();
        }
    }

    #[test]
    fn client_intent_schema_v1_round_trips_without_losing_execution_authority() {
        let intent = ClientIntent::v1(
            ClientSurface::Cli,
            PrincipalId("local-uid:1000".into()),
            "corr-1",
            ClientCommand::SubmitPrompt(SubmitPromptIntent {
                content: "verify the workspace".into(),
                session_id: Some(SessionId("session-1".into())),
                workspace: WorkspacePolicy::from_resolved_roots(
                    "/tmp/project".into(),
                    vec!["/tmp/shared".into()],
                )
                .unwrap(),
                requirements: vec![TurnRequirement::InvokeAgentRuntime {
                    runtime_id: "reviewer".into(),
                }],
                task_kind: Some(TaskKind::Coding),
                permission_mode: HostPermissionMode::Developer,
            }),
        );

        let encoded = serde_json::to_value(&intent).unwrap();
        assert_eq!(encoded["schema_version"], CLIENT_INTENT_SCHEMA_V1);
        assert_eq!(encoded["surface"], "cli");
        assert_eq!(encoded["principal"], "local-uid:1000");
        assert_eq!(
            encoded["command"]["arguments"]["workspace"]["cwd"],
            "/tmp/project"
        );
        assert_eq!(
            encoded["command"]["arguments"]["permission_mode"],
            "developer"
        );

        let decoded: ClientIntent = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded, intent);
    }
}
