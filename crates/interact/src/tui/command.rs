/// Command parser for /command input.

/// Built-in commands.
#[derive(Debug, Clone, PartialEq)]
pub enum BuiltinCommand {
    Help,
    New,
    Clear,
    Status,
    Quit,
    Input,
    Copy,
    Reflect,
    ReflectNow,
    Evolution,
    Genome,
    Computer {
        args: String,
    },
    Sessions,
    Resume {
        id: String,
    },
    Fork,
    Compact,
    Model,
    Permissions,
    Mode {
        name: String,
    },
    Plan,
    Approve,
    Agents,
    AgentDetail {
        id: String,
    },
    Hooks,
    Skills,
    SkillRun {
        name: String,
        args: String,
    },
    Interrupt,
    Context,
    Profile,
    ProfileSet {
        name: String,
    },
    Diff,
    Mention {
        path: String,
    },
    /// Query session memory (facts, recall, core blocks).
    Memory,
    MemorySearch {
        query: String,
    },
    MemoryStatus,
}

/// Parsed command type.
#[derive(Debug, Clone)]
pub enum CommandType {
    /// Built-in command (no arguments).
    Builtin(BuiltinCommand),
    /// Skill-based command with arguments.
    Skill { name: String, args: String },
    /// Slash token that is neither a registered builtin nor a daemon Skill.
    Unknown {
        name: String,
        args: String,
        suggestions: Vec<String>,
    },
}

/// Parse input starting with '/' into a CommandType.
/// Returns None if input doesn't start with '/'.
pub fn parse_command(input: &str) -> Option<CommandType> {
    super::registry::CommandRegistry::new().parse(input)
}

/// Decide whether input should be treated as a slash command (vs. regular
/// chat). A command starts with `/` and its name token (up to the first
/// space) is non-empty and contains no `/`. This keeps absolute filesystem
/// paths like `/home/user/proj` — which also start with `/` — as chat messages
/// instead of mis-parsing them as unknown skills.
pub fn looks_like_command(input: &str) -> bool {
    let Some(rest) = input.strip_prefix('/') else {
        return false;
    };
    let name = match rest.find(' ') {
        Some(i) => &rest[..i],
        None => rest,
    };
    !name.is_empty() && !name.contains('/')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_help() {
        let result = parse_command("/help").unwrap();
        assert!(matches!(result, CommandType::Builtin(BuiltinCommand::Help)));
    }

    #[test]
    fn test_parse_clear() {
        let result = parse_command("/clear").unwrap();
        assert!(matches!(
            result,
            CommandType::Builtin(BuiltinCommand::Clear)
        ));
    }

    #[test]
    fn test_parse_memory_subcommands() {
        assert!(matches!(
            parse_command("/memory status"),
            Some(CommandType::Builtin(BuiltinCommand::MemoryStatus))
        ));
        assert!(matches!(
            parse_command("/memory search deployment path"),
            Some(CommandType::Builtin(BuiltinCommand::MemorySearch { query }))
                if query == "deployment path"
        ));
    }

    #[test]
    fn test_parse_skill_no_args() {
        let result = parse_command("/code-review").unwrap();
        match result {
            CommandType::Unknown { name, args, .. } => {
                assert_eq!(name, "code-review");
                assert_eq!(args, "");
            }
            _ => panic!("Expected unknown command"),
        }
    }

    #[test]
    fn test_parse_skill_with_args() {
        let result = parse_command("/code-review main.rs").unwrap();
        match result {
            CommandType::Unknown { name, args, .. } => {
                assert_eq!(name, "code-review");
                assert_eq!(args, "main.rs");
            }
            _ => panic!("Expected unknown command"),
        }
    }

    #[test]
    fn test_parse_not_command() {
        assert!(parse_command("hello").is_none());
    }

    #[test]
    fn test_looks_like_command_true_for_commands() {
        assert!(looks_like_command("/help"));
        assert!(looks_like_command("/code-review main.rs"));
        assert!(looks_like_command("/resume abc123"));
    }

    #[test]
    fn test_looks_like_command_false_for_paths() {
        // Absolute paths start with '/' but must be treated as chat, not skills.
        assert!(!looks_like_command("/tmp/aletheon-project 分析这个项目"));
        assert!(!looks_like_command("/usr/bin/foo"));
        assert!(!looks_like_command("/home/user/proj"));
    }

    #[test]
    fn test_looks_like_command_false_for_non_slash_and_bare_slash() {
        assert!(!looks_like_command("hello"));
        assert!(!looks_like_command("/"));
        assert!(!looks_like_command("/ leading space"));
    }

    #[test]
    fn test_parse_quit_alias() {
        let result = parse_command("/exit").unwrap();
        assert!(matches!(result, CommandType::Builtin(BuiltinCommand::Quit)));
    }

    #[test]
    fn test_parse_reflect() {
        let result = parse_command("/reflect").unwrap();
        assert!(matches!(
            result,
            CommandType::Builtin(BuiltinCommand::Reflect)
        ));
    }

    #[test]
    fn test_parse_reflect_alias() {
        let result = parse_command("/r").unwrap();
        assert!(matches!(
            result,
            CommandType::Builtin(BuiltinCommand::Reflect)
        ));
    }

    #[test]
    fn test_parse_reflect_now() {
        let result = parse_command("/reflect_now").unwrap();
        assert!(matches!(
            result,
            CommandType::Builtin(BuiltinCommand::ReflectNow)
        ));
    }

    #[test]
    fn test_parse_reflect_now_alias() {
        let result = parse_command("/rn").unwrap();
        assert!(matches!(
            result,
            CommandType::Builtin(BuiltinCommand::ReflectNow)
        ));
    }

    #[test]
    fn test_parse_evolution() {
        let result = parse_command("/evolution").unwrap();
        assert!(matches!(
            result,
            CommandType::Builtin(BuiltinCommand::Evolution)
        ));
    }

    #[test]
    fn test_parse_evolution_alias() {
        let result = parse_command("/evo").unwrap();
        assert!(matches!(
            result,
            CommandType::Builtin(BuiltinCommand::Evolution)
        ));
    }

    #[test]
    fn test_parse_genome() {
        let result = parse_command("/genome").unwrap();
        assert!(matches!(
            result,
            CommandType::Builtin(BuiltinCommand::Genome)
        ));
    }

    #[test]
    fn test_parse_genome_alias() {
        let result = parse_command("/gene").unwrap();
        assert!(matches!(
            result,
            CommandType::Builtin(BuiltinCommand::Genome)
        ));
    }

    #[test]
    fn test_parse_status() {
        let result = parse_command("/status").unwrap();
        assert!(matches!(
            result,
            CommandType::Builtin(BuiltinCommand::Status)
        ));
    }

    #[test]
    fn test_parse_status_alias() {
        let result = parse_command("/st").unwrap();
        assert!(matches!(
            result,
            CommandType::Builtin(BuiltinCommand::Status)
        ));
    }

    #[test]
    fn test_parse_sessions() {
        let result = parse_command("/sessions").unwrap();
        assert!(matches!(
            result,
            CommandType::Builtin(BuiltinCommand::Sessions)
        ));
    }

    #[test]
    fn test_parse_sessions_alias() {
        let result = parse_command("/sess").unwrap();
        assert!(matches!(
            result,
            CommandType::Builtin(BuiltinCommand::Sessions)
        ));
    }

    #[test]
    fn test_parse_resume() {
        let result = parse_command("/resume abc123").unwrap();
        match result {
            CommandType::Builtin(BuiltinCommand::Resume { id }) => {
                assert_eq!(id, "abc123");
            }
            _ => panic!("Expected Resume"),
        }
    }

    #[test]
    fn test_parse_compact() {
        let result = parse_command("/compact").unwrap();
        assert!(matches!(
            result,
            CommandType::Builtin(BuiltinCommand::Compact)
        ));
    }

    #[test]
    fn test_parse_compact_alias() {
        let result = parse_command("/cmp").unwrap();
        assert!(matches!(
            result,
            CommandType::Builtin(BuiltinCommand::Compact)
        ));
    }

    #[test]
    fn test_parse_model() {
        let result = parse_command("/model").unwrap();
        assert!(matches!(
            result,
            CommandType::Builtin(BuiltinCommand::Model)
        ));
    }

    #[test]
    fn test_parse_model_alias() {
        let result = parse_command("/m").unwrap();
        assert!(matches!(
            result,
            CommandType::Builtin(BuiltinCommand::Model)
        ));
    }
}
