/// Command parser for /command input.

/// Built-in commands.
#[derive(Debug, Clone, PartialEq)]
pub enum BuiltinCommand {
    Help,
    Clear,
    Status,
    Quit,
    Input,
    Copy,
    Reflect,
    Evolution,
    Genome,
    Computer { args: String },
}

/// Parsed command type.
#[derive(Debug, Clone)]
pub enum CommandType {
    /// Built-in command (no arguments).
    Builtin(BuiltinCommand),
    /// Skill-based command with arguments.
    Skill { name: String, args: String },
}

/// Parse input starting with '/' into a CommandType.
/// Returns None if input doesn't start with '/'.
pub fn parse_command(input: &str) -> Option<CommandType> {
    let text = input.strip_prefix('/')?;
    let (name, args) = match text.find(' ') {
        Some(i) => (&text[..i], text[i + 1..].trim()),
        None => (text, ""),
    };

    match name {
        "help" => Some(CommandType::Builtin(BuiltinCommand::Help)),
        "clear" => Some(CommandType::Builtin(BuiltinCommand::Clear)),
        "status" => Some(CommandType::Builtin(BuiltinCommand::Status)),
        "quit" | "exit" => Some(CommandType::Builtin(BuiltinCommand::Quit)),
        "input" | "i" => Some(CommandType::Builtin(BuiltinCommand::Input)),
        "copy" | "cp" => Some(CommandType::Builtin(BuiltinCommand::Copy)),
        "reflect" | "r" => Some(CommandType::Builtin(BuiltinCommand::Reflect)),
        "evolution" | "evo" => Some(CommandType::Builtin(BuiltinCommand::Evolution)),
        "genome" | "gene" => Some(CommandType::Builtin(BuiltinCommand::Genome)),
        "computer" => Some(CommandType::Builtin(BuiltinCommand::Computer {
            args: args.to_string(),
        })),
        _ => Some(CommandType::Skill {
            name: name.to_string(),
            args: args.to_string(),
        }),
    }
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
        assert!(matches!(result, CommandType::Builtin(BuiltinCommand::Clear)));
    }

    #[test]
    fn test_parse_skill_no_args() {
        let result = parse_command("/code-review").unwrap();
        match result {
            CommandType::Skill { name, args } => {
                assert_eq!(name, "code-review");
                assert_eq!(args, "");
            }
            _ => panic!("Expected Skill"),
        }
    }

    #[test]
    fn test_parse_skill_with_args() {
        let result = parse_command("/code-review main.rs").unwrap();
        match result {
            CommandType::Skill { name, args } => {
                assert_eq!(name, "code-review");
                assert_eq!(args, "main.rs");
            }
            _ => panic!("Expected Skill"),
        }
    }

    #[test]
    fn test_parse_not_command() {
        assert!(parse_command("hello").is_none());
    }

    #[test]
    fn test_parse_quit_alias() {
        let result = parse_command("/exit").unwrap();
        assert!(matches!(result, CommandType::Builtin(BuiltinCommand::Quit)));
    }

    #[test]
    fn test_parse_reflect() {
        let result = parse_command("/reflect").unwrap();
        assert!(matches!(result, CommandType::Builtin(BuiltinCommand::Reflect)));
    }

    #[test]
    fn test_parse_reflect_alias() {
        let result = parse_command("/r").unwrap();
        assert!(matches!(result, CommandType::Builtin(BuiltinCommand::Reflect)));
    }

    #[test]
    fn test_parse_evolution() {
        let result = parse_command("/evolution").unwrap();
        assert!(matches!(result, CommandType::Builtin(BuiltinCommand::Evolution)));
    }

    #[test]
    fn test_parse_evolution_alias() {
        let result = parse_command("/evo").unwrap();
        assert!(matches!(result, CommandType::Builtin(BuiltinCommand::Evolution)));
    }

    #[test]
    fn test_parse_genome() {
        let result = parse_command("/genome").unwrap();
        assert!(matches!(result, CommandType::Builtin(BuiltinCommand::Genome)));
    }

    #[test]
    fn test_parse_genome_alias() {
        let result = parse_command("/gene").unwrap();
        assert!(matches!(result, CommandType::Builtin(BuiltinCommand::Genome)));
    }
}
