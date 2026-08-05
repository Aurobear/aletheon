//! Unified command registry — the single source of truth for parsing, help,
//! completion, availability, and dynamically discovered Skill commands.

use std::collections::{BTreeMap, BTreeSet};

use super::command::{BuiltinCommand, CommandType};
use fabric::contract::command::{
    command_specs, CommandAvailability as SpecAvailability, CommandExecution, CommandSpec,
    CommandSurface, CommandVisibility,
};

const RETIRED_GOVERNANCE_COMMAND_NAMES: &[&str] = &[
    "reflect",
    "r",
    "reflect_now",
    "rn",
    "evolution",
    "evo",
    "genome",
    "gene",
    "hooks",
    "hk",
    "task",
    "evaluation",
    "eval",
    "approve",
    "a",
    "plan",
    "p",
    "computer",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandExecutor {
    Local,
    Rpc,
    Skill,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandAvailability {
    Always,
    IdleOnly,
    ActiveTurnOnly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandSource {
    Builtin,
    Skill {
        skill_id: String,
        extension_id: Option<String>,
    },
}

#[derive(Debug, Clone)]
pub struct CommandDescriptor {
    pub name: String,
    pub aliases: Vec<String>,
    pub description: String,
    pub category: String,
    pub usage: String,
    pub executor: CommandExecutor,
    pub availability: CommandAvailability,
    pub acceptance_case_id: String,
    pub source: CommandSource,
    builtin_key: Option<String>,
}

impl CommandDescriptor {
    fn builtin(spec: &CommandSpec) -> Self {
        Self {
            name: spec.name.clone(),
            aliases: spec.aliases.clone(),
            description: spec.summary.clone(),
            category: spec.category.clone(),
            usage: spec.usage.clone(),
            executor: match spec.execution {
                CommandExecution::Local => CommandExecutor::Local,
                CommandExecution::Application | CommandExecution::Compatibility => {
                    CommandExecutor::Rpc
                }
            },
            availability: match spec.availability {
                SpecAvailability::Always => CommandAvailability::Always,
                SpecAvailability::IdleOnly => CommandAvailability::IdleOnly,
                SpecAvailability::ActiveTurnOnly => CommandAvailability::ActiveTurnOnly,
            },
            acceptance_case_id: format!("tui.command.{}", spec.name),
            source: CommandSource::Builtin,
            builtin_key: Some(spec.key.clone()),
        }
    }

    pub fn available(&self, turn_active: bool) -> bool {
        match self.availability {
            CommandAvailability::Always => true,
            CommandAvailability::IdleOnly => !turn_active,
            CommandAvailability::ActiveTurnOnly => turn_active,
        }
    }
}

pub struct CommandRegistry {
    builtins: Vec<CommandDescriptor>,
    skills: Vec<CommandDescriptor>,
    diagnostics: Vec<String>,
    stale: bool,
}

impl Default for CommandRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandRegistry {
    pub fn new() -> Self {
        let mut builtins = command_specs(CommandSurface::Tui)
            .filter(|spec| spec.parent.is_none() && spec.visibility != CommandVisibility::Internal)
            .map(CommandDescriptor::builtin)
            .collect::<Vec<_>>();
        builtins.sort_by(|left, right| left.name.cmp(&right.name));
        Self {
            builtins,
            skills: Vec::new(),
            diagnostics: Vec::new(),
            stale: false,
        }
    }

    pub fn parse(&self, input: &str) -> Option<CommandType> {
        let text = input.strip_prefix('/')?;
        let (name, args) = text
            .split_once(' ')
            .map_or((text, ""), |(name, args)| (name, args.trim()));
        let descriptor = self.resolve(name);
        match descriptor {
            Some(descriptor) => match &descriptor.source {
                CommandSource::Skill { skill_id, .. } => Some(CommandType::Skill {
                    name: skill_id.clone(),
                    args: args.into(),
                }),
                CommandSource::Builtin => descriptor
                    .builtin_key
                    .as_deref()
                    .and_then(|key| to_builtin(key, args))
                    .map(CommandType::Builtin),
            },
            None => Some(CommandType::Unknown {
                name: name.into(),
                args: args.into(),
                suggestions: self.suggest(name, 3),
            }),
        }
    }

    pub fn find(&self, query: &str) -> Vec<&CommandDescriptor> {
        let query = query.trim_start_matches('/').to_lowercase();
        let mut results: Vec<(u8, &CommandDescriptor)> = self
            .all()
            .filter_map(|command| {
                let name = command.name.to_lowercase();
                let alias_prefix = command
                    .aliases
                    .iter()
                    .any(|alias| alias.to_lowercase().starts_with(&query));
                let description = command.description.to_lowercase();
                let category = command.category.to_lowercase();
                let score = if query.is_empty() || name.starts_with(&query) {
                    0
                } else if alias_prefix {
                    1
                } else if category.starts_with(&query) {
                    2
                } else if description.contains(&query) {
                    3
                } else if fuzzy_subsequence(&name, &query) {
                    4
                } else {
                    return None;
                };
                Some((score, command))
            })
            .collect();
        results.sort_by(|(left_score, left), (right_score, right)| {
            left_score
                .cmp(right_score)
                .then_with(|| left.name.cmp(&right.name))
        });
        results.into_iter().map(|(_, command)| command).collect()
    }

    pub fn suggest(&self, query: &str, limit: usize) -> Vec<String> {
        self.find(query)
            .into_iter()
            .take(limit)
            .map(|command| format!("/{}", command.name))
            .collect()
    }

    pub fn resolve(&self, name: &str) -> Option<&CommandDescriptor> {
        self.all().find(|command| {
            command.name == name || command.aliases.iter().any(|alias| alias == name)
        })
    }

    pub fn is_builtin(&self, name: &str) -> bool {
        self.builtins.iter().any(|command| {
            command.name == name || command.aliases.iter().any(|alias| alias == name)
        })
    }

    pub fn is_skill(&self, name: &str) -> bool {
        self.skills.iter().any(|command| {
            command.name == name || command.aliases.iter().any(|alias| alias == name)
        })
    }

    pub fn help_text(&self) -> String {
        let mut text = String::from("Aletheon 命令：\n");
        let mut categories: BTreeMap<&str, Vec<&CommandDescriptor>> = BTreeMap::new();
        for command in &self.builtins {
            categories
                .entry(&command.category)
                .or_default()
                .push(command);
        }
        for category in ["会话", "自省", "动作", "信息"] {
            if let Some(commands) = categories.get(category) {
                text.push_str(&format!("\n── {category} ──\n"));
                for command in commands {
                    text.push_str(&format!(
                        "  {:<24} {}\n",
                        command.usage, command.description
                    ));
                }
            }
        }
        if !self.skills.is_empty() {
            text.push_str("\n── 技能 (daemon) ──\n");
            for skill in &self.skills {
                text.push_str(&format!("  /{:<22} {}\n", skill.name, skill.description));
            }
        }
        if self.stale {
            text.push_str("\n⚠ Skill catalog is stale; using last-known-good entries.\n");
        }
        for diagnostic in &self.diagnostics {
            text.push_str(&format!("\n⚠ {diagnostic}"));
        }
        text
    }

    pub fn completion_list(&self) -> Vec<String> {
        self.all()
            .map(|command| format!("/{}", command.name))
            .collect()
    }

    pub fn set_skills_from_json(&mut self, value: &serde_json::Value) {
        let Some(entries) = value.as_array() else {
            self.stale = true;
            return;
        };
        let mut reserved: BTreeSet<String> = self
            .builtins
            .iter()
            .flat_map(|command| {
                std::iter::once(command.name.clone()).chain(command.aliases.clone())
            })
            .collect();
        reserved.extend(
            RETIRED_GOVERNANCE_COMMAND_NAMES
                .iter()
                .map(|name| (*name).to_string()),
        );
        let mut skills = Vec::new();
        let mut diagnostics = Vec::new();
        let mut seen = BTreeSet::new();
        let mut name_counts = BTreeMap::<String, usize>::new();
        for entry in entries {
            if entry
                .get("enabled")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true)
            {
                if let Some(name) = entry.get("name").and_then(serde_json::Value::as_str) {
                    *name_counts.entry(name.to_owned()).or_default() += 1;
                }
            }
        }
        for entry in entries {
            if !entry
                .get("enabled")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true)
            {
                continue;
            }
            let Some(name) = entry
                .get("name")
                .and_then(serde_json::Value::as_str)
                .filter(|name| !name.is_empty())
            else {
                continue;
            };
            let skill_id = entry
                .get("id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(name);
            let extension_id = entry
                .get("extension_id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned);
            let mut command_name = name.to_owned();
            if reserved.contains(name)
                || name_counts.get(name).copied().unwrap_or_default() > 1
                || !seen.insert(name.to_owned())
            {
                if let Some(extension) = &extension_id {
                    command_name = format!("{extension}:{name}");
                } else {
                    diagnostics.push(format!(
                        "Skill command /{name} conflicts and has no extension namespace"
                    ));
                    continue;
                }
            }
            skills.push(CommandDescriptor {
                name: command_name.clone(),
                aliases: Vec::new(),
                description: entry
                    .get("description")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("No description")
                    .into(),
                category: "技能".into(),
                usage: format!("/{command_name} [args]"),
                executor: CommandExecutor::Skill,
                availability: CommandAvailability::IdleOnly,
                acceptance_case_id: format!("tui.skill.{skill_id}"),
                source: CommandSource::Skill {
                    skill_id: skill_id.into(),
                    extension_id,
                },
                builtin_key: None,
            });
        }
        skills.sort_by(|left, right| left.name.cmp(&right.name));
        self.skills = skills;
        self.diagnostics = diagnostics;
        self.stale = false;
    }

    pub fn builtins(&self) -> &[CommandDescriptor] {
        &self.builtins
    }
    pub fn skills(&self) -> &[CommandDescriptor] {
        &self.skills
    }
    pub fn diagnostics(&self) -> &[String] {
        &self.diagnostics
    }
    pub fn is_stale(&self) -> bool {
        self.stale
    }
    fn all(&self) -> impl Iterator<Item = &CommandDescriptor> {
        self.builtins.iter().chain(&self.skills)
    }
}

fn to_builtin(key: &str, args: &str) -> Option<BuiltinCommand> {
    Some(match key {
        "tui.help" => BuiltinCommand::Help,
        "tui.new" => BuiltinCommand::New,
        "tui.clear" => BuiltinCommand::Clear,
        "tui.status" => BuiltinCommand::Status,
        "tui.compact" => BuiltinCommand::Compact,
        "tui.sessions" => BuiltinCommand::Sessions,
        "tui.resume" => BuiltinCommand::Resume { id: args.into() },
        "tui.fork" => BuiltinCommand::Fork,
        "tui.rewind" => BuiltinCommand::Rewind {
            prompt_index: args.into(),
        },
        "tui.model" => BuiltinCommand::Model,
        "tui.permissions" => BuiltinCommand::Permissions,
        "tui.context" => BuiltinCommand::Context,
        "tui.interrupt" => BuiltinCommand::Interrupt,
        "tui.copy" => BuiltinCommand::Copy,
        "tui.mode" => BuiltinCommand::Mode { name: args.into() },
        "tui.quit" => BuiltinCommand::Quit,
        "tui.agents" => BuiltinCommand::Agents,
        "tui.agent" => BuiltinCommand::AgentDetail { id: args.into() },
        "tui.skills" => BuiltinCommand::Skills,
        "tui.profile" => {
            if args.is_empty() {
                BuiltinCommand::Profile
            } else {
                BuiltinCommand::ProfileSet { name: args.into() }
            }
        }
        "tui.diff" => BuiltinCommand::Diff,
        "tui.mention" => BuiltinCommand::Mention { path: args.into() },
        "tui.input" => BuiltinCommand::Input,
        "tui.memory" => {
            if args == "status" {
                BuiltinCommand::MemoryStatus
            } else if let Some(query) = args.strip_prefix("search ").map(str::trim) {
                BuiltinCommand::MemorySearch {
                    query: query.to_string(),
                }
            } else {
                BuiltinCommand::Memory
            }
        }
        _ => return None,
    })
}

fn fuzzy_subsequence(candidate: &str, query: &str) -> bool {
    let mut chars = candidate.chars();
    query
        .chars()
        .all(|needle| chars.by_ref().any(|value| value == needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_names_aliases_and_acceptance_ids_are_unique() {
        let registry = CommandRegistry::new();
        let mut names = BTreeSet::new();
        let mut cases = BTreeSet::new();
        for command in registry.builtins() {
            assert!(
                names.insert(command.name.clone()),
                "duplicate {}",
                command.name
            );
            for alias in &command.aliases {
                assert!(names.insert(alias.clone()), "duplicate alias {alias}");
            }
            assert!(cases.insert(command.acceptance_case_id.clone()));
        }
    }

    #[test]
    fn public_builtin_command_set_is_exact() {
        let registry = CommandRegistry::new();
        let actual = registry
            .builtins()
            .iter()
            .map(|command| command.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            actual,
            vec![
                "agent",
                "agents",
                "clear",
                "compact",
                "context",
                "copy",
                "diff",
                "fork",
                "help",
                "input",
                "interrupt",
                "memory",
                "mention",
                "mode",
                "model",
                "new",
                "permissions",
                "profile",
                "quit",
                "resume",
                "rewind",
                "sessions",
                "skills",
                "status",
            ]
        );
        for retired in RETIRED_GOVERNANCE_COMMAND_NAMES {
            assert!(
                !registry.is_builtin(retired),
                "retired /{retired} remains public"
            );
        }
    }

    #[test]
    fn required_session_commands_are_registered() {
        let registry = CommandRegistry::new();
        for name in [
            "help",
            "new",
            "clear",
            "compact",
            "status",
            "model",
            "permissions",
            "sessions",
            "resume",
            "fork",
            "diff",
            "mention",
            "skills",
            "agents",
            "interrupt",
            "copy",
            "quit",
            "mode",
            "context",
            "profile",
            "memory",
        ] {
            assert!(registry.is_builtin(name), "missing /{name}");
        }
    }

    #[test]
    fn registry_parses_builtin_alias_and_rejects_unknown() {
        let registry = CommandRegistry::new();
        assert!(matches!(
            registry.parse("/st"),
            Some(CommandType::Builtin(BuiltinCommand::Status))
        ));
        assert!(matches!(
            registry.parse("/unknown"),
            Some(CommandType::Unknown { .. })
        ));
    }

    #[test]
    fn u_cli_002_compatibility_aliases_forward_to_the_canonical_handler() {
        let registry = CommandRegistry::new();
        for (canonical, compatibility) in [
            ("/status", "/st"),
            ("/compact", "/cmp"),
            ("/sessions", "/sess"),
            ("/model", "/m"),
            ("/context", "/ctx"),
            ("/interrupt", "/int"),
            ("/copy", "/cp"),
            ("/quit", "/exit"),
            ("/agents", "/ag"),
            ("/skills", "/sk"),
            ("/profile", "/prof"),
            ("/input", "/i"),
        ] {
            assert_eq!(
                registry.parse(canonical),
                registry.parse(compatibility),
                "{compatibility} diverged from {canonical}"
            );
        }
    }

    #[test]
    fn skill_conflicts_require_namespace() {
        let mut registry = CommandRegistry::new();
        registry.set_skills_from_json(&serde_json::json!([
            {"id":"skill.help","name":"help","extension_id":"pack","enabled":true},
            {"id":"skill.review","name":"review","enabled":true}
        ]));
        assert!(registry.is_skill("pack:help"));
        assert!(registry.is_skill("review"));
        assert!(
            matches!(registry.parse("/review src"), Some(CommandType::Skill { name, args }) if name == "skill.review" && args == "src")
        );
    }

    #[test]
    fn retired_governance_names_cannot_be_reintroduced_by_skills() {
        let mut registry = CommandRegistry::new();
        registry.set_skills_from_json(&serde_json::json!([
            {"id":"skill.reflect","name":"reflect","extension_id":"pack","enabled":true},
            {"id":"skill.hooks","name":"hooks","enabled":true}
        ]));

        assert!(registry.is_skill("pack:reflect"));
        assert!(!registry.is_skill("reflect"));
        assert!(!registry.is_skill("hooks"));
        assert!(matches!(
            registry.parse("/reflect"),
            Some(CommandType::Unknown { .. })
        ));
        assert!(matches!(
            registry.parse("/hooks"),
            Some(CommandType::Unknown { .. })
        ));
    }

    #[test]
    fn fuzzy_find_ranks_prefix_before_subsequence() {
        let registry = CommandRegistry::new();
        let results = registry.find("cp");
        assert_eq!(
            results.first().map(|command| command.name.as_str()),
            Some("copy")
        );
    }

    #[test]
    fn action_palette_searches_description_alias_and_category() {
        let registry = CommandRegistry::new();
        assert!(!registry.find("session").is_empty());
        assert!(!registry.find("resume").is_empty());
        let alias = registry
            .all()
            .find_map(|command| command.aliases.first())
            .expect("at least one alias");
        assert!(!registry.find(alias).is_empty());
    }
}
