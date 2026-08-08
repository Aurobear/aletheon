//! Model-visible tool projection for one cognitive turn.
//!
//! The active Agent profile remains the execution authority. This module only
//! controls which authorized JSON schemas are sent to the model initially;
//! `tool_search` can expand that visible set from the same immutable authority
//! snapshot during the turn.

use fabric::{TaskKind, ToolDefinition, TurnRequirement};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

/// Small, stable starter surface for ordinary turns. These tools let the model
/// inspect an unfamiliar workspace, retrieve evidence, ask for clarification,
/// and discover any less-common authorized capability without paying the token
/// and selection cost of the complete catalog on every conversational turn.
const GENERAL_STARTER_TOOLS: &[&str] = &[
    "artifact_read",
    "file_read",
    "file_search",
    "glob",
    "grep",
    "repo_inspect",
    "request_user_input",
    "system_status",
    "tool_search",
    "toolchain_status",
    "web_fetch",
    "web_search",
];

/// Explicit typed coding work receives the complete common engineering
/// surface immediately. Rare package/platform tools still arrive through
/// `tool_search`, so broad authority does not imply a giant stable schema.
const CODING_STARTER_TOOLS: &[&str] = &[
    "agent_cancel",
    "agent_list",
    "agent_send",
    "agent_spawn",
    "agent_wait",
    "apply_patch",
    "bash_exec",
    "code_graph",
    "exec_command",
    "file_write",
    "git_add",
    "git_branch",
    "git_commit",
    "git_diff",
    "git_log",
    "git_push",
    "git_reset",
    "git_restore",
    "git_show",
    "git_stash",
    "git_status",
    "task_create",
    "task_get",
    "task_list",
    "task_update",
    "validation_run",
    "write_stdin",
];

const AGENT_RUNTIME_TOOLS: &[&str] = &[
    "agent_cancel",
    "agent_list",
    "agent_send",
    "agent_spawn",
    "agent_wait",
];

#[derive(Debug, Clone)]
pub struct InitialToolProjection {
    pub definitions: Vec<ToolDefinition>,
    pub authorized_count: usize,
}

impl InitialToolProjection {
    pub fn omitted_count(&self) -> usize {
        self.authorized_count.saturating_sub(self.definitions.len())
    }
}

/// Select the deterministic initial schema set from typed task state and the
/// immutable authorized catalog. If `tool_search` is unavailable, fail open to
/// the full catalog so projection can never strand an Agent with unreachable
/// authority.
pub fn project_initial_tools(
    authorized: &[ToolDefinition],
    task_kind: Option<TaskKind>,
    requirements: &[TurnRequirement],
) -> InitialToolProjection {
    let authorized_count = authorized.len();
    if !authorized.iter().any(|tool| tool.name == "tool_search") {
        return InitialToolProjection {
            definitions: authorized.to_vec(),
            authorized_count,
        };
    }

    let mut visible = GENERAL_STARTER_TOOLS
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<BTreeSet<_>>();
    if task_kind == Some(TaskKind::Coding) {
        visible.extend(CODING_STARTER_TOOLS.iter().map(|name| (*name).to_owned()));
    }
    for requirement in requirements {
        match requirement {
            TurnRequirement::InvokeCapability { name } => {
                visible.insert(name.clone());
            }
            TurnRequirement::InvokeAgentRuntime { .. } => {
                visible.extend(AGENT_RUNTIME_TOOLS.iter().map(|name| (*name).to_owned()));
            }
            TurnRequirement::ObserveTerminal { .. } | TurnRequirement::RunRoleGraph { .. } => {}
        }
    }

    InitialToolProjection {
        definitions: authorized
            .iter()
            .filter(|definition| visible.contains(&definition.name))
            .cloned()
            .collect(),
        authorized_count,
    }
}

/// Immutable lookup used after a successful `tool_search`. Search output is
/// untrusted model-visible data, so only names that resolve back into this
/// Host-authorized catalog can become visible on the next inference round.
#[derive(Debug, Clone)]
pub struct AuthorizedToolCatalog {
    by_name: BTreeMap<String, ToolDefinition>,
}

#[derive(Debug, Deserialize)]
struct ToolSearchOutput {
    #[serde(default)]
    matches: Vec<ToolSearchMatch>,
}

#[derive(Debug, Deserialize)]
struct ToolSearchMatch {
    name: String,
}

impl AuthorizedToolCatalog {
    pub fn new(definitions: &[ToolDefinition]) -> Self {
        Self {
            by_name: definitions
                .iter()
                .cloned()
                .map(|definition| (definition.name.clone(), definition))
                .collect(),
        }
    }

    pub fn resolve_search_output(&self, output: &str) -> Vec<ToolDefinition> {
        let Ok(output) = serde_json::from_str::<ToolSearchOutput>(output) else {
            return Vec::new();
        };
        output
            .matches
            .into_iter()
            .filter_map(|matched| self.by_name.get(&matched.name).cloned())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tool(name: &str) -> ToolDefinition {
        ToolDefinition {
            name: name.into(),
            description: format!("{name} fixture"),
            input_schema: json!({"type": "object"}),
        }
    }

    #[test]
    fn ordinary_turn_starts_narrow_without_losing_discovery() {
        let authorized = vec![
            tool("tool_search"),
            tool("file_read"),
            tool("apply_patch"),
            tool("robot_episode_run"),
        ];
        let projection = project_initial_tools(&authorized, None, &[]);
        let names = projection
            .definitions
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["tool_search", "file_read"]);
        assert_eq!(projection.omitted_count(), 2);
    }

    #[test]
    fn typed_coding_and_explicit_requirements_expand_immediately() {
        let authorized = vec![
            tool("tool_search"),
            tool("file_read"),
            tool("apply_patch"),
            tool("robot_episode_run"),
        ];
        let projection = project_initial_tools(
            &authorized,
            Some(TaskKind::Coding),
            &[TurnRequirement::InvokeCapability {
                name: "robot_episode_run".into(),
            }],
        );
        let names = projection
            .definitions
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                "tool_search",
                "file_read",
                "apply_patch",
                "robot_episode_run"
            ]
        );
    }

    #[test]
    fn search_activation_is_attenuated_by_authorized_catalog() {
        let catalog = AuthorizedToolCatalog::new(&[tool("file_read"), tool("robot_episode_run")]);
        let activated = catalog.resolve_search_output(
            &json!({
                "matches": [
                    {"name": "robot_episode_run"},
                    {"name": "not_authorized"}
                ]
            })
            .to_string(),
        );
        assert_eq!(activated.len(), 1);
        assert_eq!(activated[0].name, "robot_episode_run");
    }

    #[test]
    fn missing_search_capability_fails_open() {
        let authorized = vec![tool("file_read"), tool("apply_patch")];
        let projection = project_initial_tools(&authorized, None, &[]);
        assert_eq!(projection.definitions.len(), authorized.len());
    }
}
