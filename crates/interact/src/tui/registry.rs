//! Unified command registry — the single source of truth for parsing, help,
//! completion, availability, and dynamically discovered Skill commands.

use std::collections::{BTreeMap, BTreeSet};

use super::command::{BuiltinCommand, CommandType};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BuiltinId {
    Help,
    New,
    Clear,
    Status,
    Compact,
    Sessions,
    Resume,
    Fork,
    Model,
    Permissions,
    Context,
    Interrupt,
    Reflect,
    ReflectNow,
    Evolution,
    Genome,
    Plan,
    Copy,
    Mode,
    Approve,
    Quit,
    Agents,
    Agent,
    Hooks,
    Skills,
    Profile,
    Computer,
    Diff,
    Mention,
    Input,
    Memory,
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
    builtin: Option<BuiltinId>,
}

impl CommandDescriptor {
    fn builtin(
        name: &str,
        aliases: &[&str],
        description: &str,
        category: &str,
        usage: &str,
        executor: CommandExecutor,
        availability: CommandAvailability,
        id: BuiltinId,
    ) -> Self {
        Self {
            name: name.into(),
            aliases: aliases.iter().map(|value| (*value).into()).collect(),
            description: description.into(),
            category: category.into(),
            usage: usage.into(),
            executor,
            availability,
            acceptance_case_id: format!("tui.command.{name}"),
            source: CommandSource::Builtin,
            builtin: Some(id),
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
        use BuiltinId as B;
        use CommandAvailability::{ActiveTurnOnly as Active, Always, IdleOnly as Idle};
        use CommandExecutor::{Local, Rpc};
        let mut builtins = vec![
            CommandDescriptor::builtin(
                "help",
                &[],
                "显示帮助信息",
                "会话",
                "/help",
                Local,
                Always,
                B::Help,
            ),
            CommandDescriptor::builtin(
                "new",
                &[],
                "创建并切换到新会话",
                "会话",
                "/new",
                Rpc,
                Idle,
                B::New,
            ),
            CommandDescriptor::builtin(
                "clear",
                &[],
                "创建新会话并清屏",
                "会话",
                "/clear",
                Rpc,
                Idle,
                B::Clear,
            ),
            CommandDescriptor::builtin(
                "status",
                &["st"],
                "查看运行时状态",
                "会话",
                "/status",
                Rpc,
                Always,
                B::Status,
            ),
            CommandDescriptor::builtin(
                "compact",
                &["cmp"],
                "压缩上下文",
                "会话",
                "/compact",
                Rpc,
                Idle,
                B::Compact,
            ),
            CommandDescriptor::builtin(
                "sessions",
                &["sess"],
                "列出历史会话",
                "会话",
                "/sessions",
                Rpc,
                Always,
                B::Sessions,
            ),
            CommandDescriptor::builtin(
                "resume",
                &[],
                "恢复指定会话",
                "会话",
                "/resume <id>",
                Rpc,
                Idle,
                B::Resume,
            ),
            CommandDescriptor::builtin(
                "fork",
                &[],
                "从当前会话创建分支",
                "会话",
                "/fork",
                Rpc,
                Idle,
                B::Fork,
            ),
            CommandDescriptor::builtin(
                "model",
                &["m"],
                "查询或切换模型",
                "会话",
                "/model",
                Rpc,
                Always,
                B::Model,
            ),
            CommandDescriptor::builtin(
                "permissions",
                &[],
                "查看权限与审批策略",
                "会话",
                "/permissions",
                Rpc,
                Always,
                B::Permissions,
            ),
            CommandDescriptor::builtin(
                "context",
                &["ctx"],
                "显示当前上下文信息",
                "会话",
                "/context",
                Local,
                Always,
                B::Context,
            ),
            CommandDescriptor::builtin(
                "interrupt",
                &["int"],
                "中断当前操作",
                "会话",
                "/interrupt",
                Rpc,
                Active,
                B::Interrupt,
            ),
            CommandDescriptor::builtin(
                "reflect",
                &["r"],
                "查看反思记录",
                "自省",
                "/reflect",
                Rpc,
                Always,
                B::Reflect,
            ),
            CommandDescriptor::builtin(
                "reflect_now",
                &["rn"],
                "执行即时反思",
                "自省",
                "/reflect_now",
                Rpc,
                Idle,
                B::ReflectNow,
            ),
            CommandDescriptor::builtin(
                "evolution",
                &["evo"],
                "查看演化历史",
                "自省",
                "/evolution",
                Rpc,
                Always,
                B::Evolution,
            ),
            CommandDescriptor::builtin(
                "genome",
                &["gene"],
                "查看当前基因组",
                "自省",
                "/genome",
                Rpc,
                Always,
                B::Genome,
            ),
            CommandDescriptor::builtin(
                "plan",
                &["p"],
                "切换 Plan 模式",
                "自省",
                "/plan",
                Rpc,
                Idle,
                B::Plan,
            ),
            CommandDescriptor::builtin(
                "copy",
                &["cp"],
                "复制最后回复到剪贴板",
                "动作",
                "/copy",
                Local,
                Always,
                B::Copy,
            ),
            CommandDescriptor::builtin(
                "mode",
                &[],
                "切换协作模式",
                "动作",
                "/mode [plan|auto|sandbox]",
                Rpc,
                Idle,
                B::Mode,
            ),
            CommandDescriptor::builtin(
                "approve",
                &["a"],
                "批准待审批操作",
                "动作",
                "/approve",
                Rpc,
                Always,
                B::Approve,
            ),
            CommandDescriptor::builtin(
                "quit",
                &["exit"],
                "退出",
                "动作",
                "/quit",
                Local,
                Always,
                B::Quit,
            ),
            CommandDescriptor::builtin(
                "agents",
                &["ag"],
                "列出活跃子 Agent",
                "信息",
                "/agents",
                Local,
                Always,
                B::Agents,
            ),
            CommandDescriptor::builtin(
                "agent",
                &[],
                "查看子 Agent 详情",
                "信息",
                "/agent <id>",
                Local,
                Always,
                B::Agent,
            ),
            CommandDescriptor::builtin(
                "hooks",
                &["hk"],
                "列出已注册 Hook",
                "信息",
                "/hooks",
                Rpc,
                Always,
                B::Hooks,
            ),
            CommandDescriptor::builtin(
                "skills",
                &["sk"],
                "列出可用 Skill",
                "信息",
                "/skills",
                Rpc,
                Always,
                B::Skills,
            ),
            CommandDescriptor::builtin(
                "profile",
                &["prof"],
                "查询或切换 Agent Profile",
                "信息",
                "/profile [name]",
                Rpc,
                Idle,
                B::Profile,
            ),
            CommandDescriptor::builtin(
                "computer",
                &[],
                "查看或控制计算机交互",
                "信息",
                "/computer [args]",
                Local,
                Always,
                B::Computer,
            ),
            CommandDescriptor::builtin(
                "diff",
                &[],
                "显示当前工作区差异",
                "信息",
                "/diff",
                Local,
                Always,
                B::Diff,
            ),
            CommandDescriptor::builtin(
                "mention",
                &[],
                "向输入加入文件引用",
                "信息",
                "/mention <path>",
                Local,
                Always,
                B::Mention,
            ),
            CommandDescriptor::builtin(
                "input",
                &["i"],
                "打开多行输入",
                "动作",
                "/input",
                Local,
                Always,
                B::Input,
            ),
            CommandDescriptor::builtin(
                "memory",
                &[],
                "查看会话记忆（核心记忆块、近期事实、回忆记录）",
                "信息",
                "/memory",
                Rpc,
                Always,
                B::Memory,
            ),
        ];
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
                    .builtin
                    .map(|id| CommandType::Builtin(to_builtin(id, args))),
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
                let score = if query.is_empty() || name.starts_with(&query) {
                    0
                } else if alias_prefix {
                    1
                } else if fuzzy_subsequence(&name, &query) {
                    2
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
        let reserved: BTreeSet<String> = self
            .builtins
            .iter()
            .flat_map(|command| {
                std::iter::once(command.name.clone()).chain(command.aliases.clone())
            })
            .collect();
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
                builtin: None,
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

fn to_builtin(id: BuiltinId, args: &str) -> BuiltinCommand {
    match id {
        BuiltinId::Help => BuiltinCommand::Help,
        BuiltinId::New => BuiltinCommand::New,
        BuiltinId::Clear => BuiltinCommand::Clear,
        BuiltinId::Status => BuiltinCommand::Status,
        BuiltinId::Compact => BuiltinCommand::Compact,
        BuiltinId::Sessions => BuiltinCommand::Sessions,
        BuiltinId::Resume => BuiltinCommand::Resume { id: args.into() },
        BuiltinId::Fork => BuiltinCommand::Fork,
        BuiltinId::Model => BuiltinCommand::Model,
        BuiltinId::Permissions => BuiltinCommand::Permissions,
        BuiltinId::Context => BuiltinCommand::Context,
        BuiltinId::Interrupt => BuiltinCommand::Interrupt,
        BuiltinId::Reflect => BuiltinCommand::Reflect,
        BuiltinId::ReflectNow => BuiltinCommand::ReflectNow,
        BuiltinId::Evolution => BuiltinCommand::Evolution,
        BuiltinId::Genome => BuiltinCommand::Genome,
        BuiltinId::Plan => BuiltinCommand::Plan,
        BuiltinId::Copy => BuiltinCommand::Copy,
        BuiltinId::Mode => BuiltinCommand::Mode { name: args.into() },
        BuiltinId::Approve => BuiltinCommand::Approve,
        BuiltinId::Quit => BuiltinCommand::Quit,
        BuiltinId::Agents => BuiltinCommand::Agents,
        BuiltinId::Agent => BuiltinCommand::AgentDetail { id: args.into() },
        BuiltinId::Hooks => BuiltinCommand::Hooks,
        BuiltinId::Skills => BuiltinCommand::Skills,
        BuiltinId::Profile => {
            if args.is_empty() {
                BuiltinCommand::Profile
            } else {
                BuiltinCommand::ProfileSet { name: args.into() }
            }
        }
        BuiltinId::Computer => BuiltinCommand::Computer { args: args.into() },
        BuiltinId::Diff => BuiltinCommand::Diff,
        BuiltinId::Mention => BuiltinCommand::Mention { path: args.into() },
        BuiltinId::Input => BuiltinCommand::Input,
        BuiltinId::Memory => BuiltinCommand::Memory,
    }
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
    fn required_production_commands_are_registered() {
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
            "hooks",
            "agents",
            "interrupt",
            "copy",
            "quit",
            "reflect",
            "reflect_now",
            "evolution",
            "genome",
            "mode",
            "plan",
            "approve",
            "context",
            "profile",
            "computer",
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
    fn fuzzy_find_ranks_prefix_before_subsequence() {
        let registry = CommandRegistry::new();
        let results = registry.find("cp");
        assert_eq!(
            results.first().map(|command| command.name.as_str()),
            Some("copy")
        );
    }
}
