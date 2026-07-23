//! Unified command registry — single source of truth for all commands.
//!
//! Replaces the four independent command lists that previously existed in
//! `command.rs` (enum), `key_handler.rs` (Tab completions), `submit.rs`
//! (`/help` text), and `help_overlay.rs` (? key overlay).

use std::collections::BTreeMap;

/// Descriptor for one user-visible command.
#[derive(Debug, Clone)]
pub struct CommandDescriptor {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub description: &'static str,
    pub category: &'static str, // "会话", "自省", "动作", "信息", "技能"
    pub usage: &'static str,
    pub has_args: bool,
}

/// Single authority for all command metadata.
pub struct CommandRegistry {
    /// Built-in commands (sorted by name).
    builtins: Vec<CommandDescriptor>,
    /// Skill commands populated from daemon RPC.
    skills: Vec<CommandDescriptor>,
}

impl CommandRegistry {
    /// Create with all built-in commands.
    pub fn new() -> Self {
        let mut builtins = vec![
            // ── 会话 (Session) ──
            CommandDescriptor {
                name: "help",
                aliases: &[],
                description: "显示帮助信息",
                category: "会话",
                usage: "/help",
                has_args: false,
            },
            CommandDescriptor {
                name: "clear",
                aliases: &[],
                description: "创建新会话并清屏",
                category: "会话",
                usage: "/clear",
                has_args: false,
            },
            CommandDescriptor {
                name: "status",
                aliases: &["st"],
                description: "查看运行时状态",
                category: "会话",
                usage: "/status",
                has_args: false,
            },
            CommandDescriptor {
                name: "compact",
                aliases: &["cmp"],
                description: "压缩上下文",
                category: "会话",
                usage: "/compact",
                has_args: false,
            },
            CommandDescriptor {
                name: "sessions",
                aliases: &["sess"],
                description: "列出历史会话",
                category: "会话",
                usage: "/sessions",
                has_args: false,
            },
            CommandDescriptor {
                name: "resume",
                aliases: &[],
                description: "恢复指定会话",
                category: "会话",
                usage: "/resume <id>",
                has_args: true,
            },
            CommandDescriptor {
                name: "model",
                aliases: &["m"],
                description: "查询可用模型",
                category: "会话",
                usage: "/model",
                has_args: false,
            },
            CommandDescriptor {
                name: "context",
                aliases: &["ctx"],
                description: "显示当前上下文信息",
                category: "会话",
                usage: "/context",
                has_args: false,
            },
            CommandDescriptor {
                name: "interrupt",
                aliases: &["int"],
                description: "中断当前操作",
                category: "会话",
                usage: "/interrupt",
                has_args: false,
            },
            // ── 自省 (Introspection) ──
            CommandDescriptor {
                name: "reflect",
                aliases: &["r"],
                description: "查看反思记录",
                category: "自省",
                usage: "/reflect",
                has_args: false,
            },
            CommandDescriptor {
                name: "reflect_now",
                aliases: &["rn"],
                description: "执行即时反思",
                category: "自省",
                usage: "/reflect_now",
                has_args: false,
            },
            CommandDescriptor {
                name: "evolution",
                aliases: &["evo"],
                description: "查看演化历史",
                category: "自省",
                usage: "/evolution",
                has_args: false,
            },
            CommandDescriptor {
                name: "genome",
                aliases: &["gene"],
                description: "查看当前基因组",
                category: "自省",
                usage: "/genome",
                has_args: false,
            },
            CommandDescriptor {
                name: "plan",
                aliases: &["p"],
                description: "切换 Plan 模式",
                category: "自省",
                usage: "/plan",
                has_args: false,
            },
            // ── 动作 (Actions) ──
            CommandDescriptor {
                name: "copy",
                aliases: &["cp"],
                description: "复制最后回复到剪贴板",
                category: "动作",
                usage: "/copy",
                has_args: false,
            },
            CommandDescriptor {
                name: "mode",
                aliases: &[],
                description: "切换协作模式",
                category: "动作",
                usage: "/mode [plan|auto|sandbox]",
                has_args: true,
            },
            CommandDescriptor {
                name: "approve",
                aliases: &["a"],
                description: "批准待审批操作",
                category: "动作",
                usage: "/approve",
                has_args: false,
            },
            CommandDescriptor {
                name: "quit",
                aliases: &["exit"],
                description: "退出",
                category: "动作",
                usage: "/quit",
                has_args: false,
            },
            // ── 信息 (Info) ──
            CommandDescriptor {
                name: "agents",
                aliases: &["ag"],
                description: "列出活跃子 Agent",
                category: "信息",
                usage: "/agents",
                has_args: false,
            },
            CommandDescriptor {
                name: "agent",
                aliases: &[],
                description: "查看子 Agent 详情",
                category: "信息",
                usage: "/agent <id>",
                has_args: true,
            },
            CommandDescriptor {
                name: "hooks",
                aliases: &["hk"],
                description: "列出已注册 Hook",
                category: "信息",
                usage: "/hooks",
                has_args: false,
            },
            CommandDescriptor {
                name: "skills",
                aliases: &["sk"],
                description: "列出可用 Skill",
                category: "信息",
                usage: "/skills",
                has_args: false,
            },
            CommandDescriptor {
                name: "skill",
                aliases: &[],
                description: "执行指定 Skill",
                category: "信息",
                usage: "/skill <name> [args]",
                has_args: true,
            },
            CommandDescriptor {
                name: "profile",
                aliases: &["prof"],
                description: "查询/切换 Agent Profile",
                category: "信息",
                usage: "/profile [name]",
                has_args: true,
            },
        ];
        builtins.sort_by(|a, b| a.name.cmp(b.name));
        Self {
            builtins,
            skills: Vec::new(),
        }
    }

    /// Find commands matching a prefix (for popup/completion).
    pub fn find(&self, prefix: &str) -> Vec<&CommandDescriptor> {
        let lower = prefix.to_lowercase();
        let mut results: Vec<&CommandDescriptor> = self
            .builtins
            .iter()
            .chain(self.skills.iter())
            .filter(|cmd| {
                cmd.name.to_lowercase().starts_with(&lower)
                    || cmd
                        .aliases
                        .iter()
                        .any(|a| a.to_lowercase().starts_with(&lower))
            })
            .collect();
        results.sort_by(|a, b| a.name.cmp(b.name));
        results
    }

    /// Exact name or alias match.
    pub fn resolve(&self, name: &str) -> Option<&CommandDescriptor> {
        self.builtins
            .iter()
            .chain(self.skills.iter())
            .find(|cmd| cmd.name == name || cmd.aliases.contains(&name))
    }

    /// Check if a name is a registered builtin command.
    pub fn is_builtin(&self, name: &str) -> bool {
        self.builtins
            .iter()
            .any(|cmd| cmd.name == name || cmd.aliases.contains(&name))
    }

    /// Check if a name is a registered skill command.
    pub fn is_skill(&self, name: &str) -> bool {
        self.skills
            .iter()
            .any(|cmd| cmd.name == name || cmd.aliases.contains(&name))
    }

    /// Generate help text from descriptors.
    pub fn help_text(&self) -> String {
        let mut text = String::from("Aletheon 命令：\n\n");
        let mut by_category: BTreeMap<&str, Vec<&CommandDescriptor>> = BTreeMap::new();
        for cmd in &self.builtins {
            by_category.entry(cmd.category).or_default().push(cmd);
        }
        let order = ["会话", "自省", "动作", "信息"];
        for cat in &order {
            if let Some(cmds) = by_category.get(cat) {
                text.push_str(&format!("\n── {} ──\n", cat));
                for cmd in cmds {
                    let alias = if cmd.aliases.is_empty() {
                        String::new()
                    } else {
                        format!(" ({})", cmd.aliases.join(", "))
                    };
                    text.push_str(&format!(
                        "  /{:<16} {}{}\n",
                        cmd.name, cmd.description, alias
                    ));
                }
            }
        }
        if !self.skills.is_empty() {
            text.push_str("\n── 技能 (daemon) ──\n");
            for sk in &self.skills {
                text.push_str(&format!("  /{:<16} {}\n", sk.name, sk.description));
            }
        }
        text.push_str("\n快捷键：Shift+Enter 换行 | Ctrl+C 取消 | PgUp/PgDn 滚动");
        text
    }

    /// Full completion list for Tab (with / prefix).
    pub fn completion_list(&self) -> Vec<String> {
        let mut list: Vec<String> = self
            .builtins
            .iter()
            .map(|cmd| format!("/{}", cmd.name))
            .collect();
        for cmd in &self.builtins {
            for alias in cmd.aliases {
                list.push(format!("/{}", *alias));
            }
        }
        for sk in &self.skills {
            list.push(format!("/{}", sk.name));
        }
        list.sort();
        list.dedup();
        list
    }

    /// Replace the skill list from daemon response.
    pub fn set_skills(&mut self, skills: Vec<CommandDescriptor>) {
        self.skills = skills;
    }

    /// List built-in command descriptors.
    pub fn builtins(&self) -> &[CommandDescriptor] {
        &self.builtins
    }
}
