//! Host-owned classification for managed shell commands.
//!
//! Model-provided labels are intentionally ignored. Only commands that this
//! module can prove read-only may bypass a repository change transaction.

use fabric::security::RiskCategory;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommandEffect {
    ReadOnly,
    ReadOnlyNetwork,
    WorkspaceMutation,
    NetworkEgress,
    SystemChange,
    Destructive,
}

impl CommandEffect {
    pub(crate) fn requires_transaction(self) -> bool {
        !matches!(self, Self::ReadOnly | Self::ReadOnlyNetwork)
    }

    pub(crate) fn risk_category(self) -> RiskCategory {
        match self {
            Self::ReadOnly => RiskCategory::ReadOnly,
            Self::ReadOnlyNetwork => RiskCategory::SystemChange,
            Self::WorkspaceMutation => RiskCategory::FileModification,
            Self::NetworkEgress | Self::SystemChange => RiskCategory::SystemChange,
            Self::Destructive => RiskCategory::Destructive,
        }
    }
}

pub(crate) fn classify_command(command: &str) -> CommandEffect {
    let normalized = command.trim();
    if normalized.is_empty() {
        return CommandEffect::WorkspaceMutation;
    }
    let normalized = strip_safe_redirections(normalized);
    let lower = normalized.to_ascii_lowercase();

    if invokes_program(&lower, &["rm", "rmdir", "mkfs", "shutdown", "reboot"])
        || lower.contains("git reset --hard")
        || lower.contains("git clean -f")
    {
        return CommandEffect::Destructive;
    }
    if invokes_program(
        &lower,
        &[
            "sudo", "su", "doas", "apt", "apt-get", "dpkg", "rpm", "dnf", "yum", "pacman",
        ],
    ) {
        return CommandEffect::SystemChange;
    }
    if invokes_program(&lower, &["systemctl"]) {
        let segments = split_shell_segments(&normalized);
        if segments.is_empty() || !segments.iter().all(|segment| is_read_only_segment(segment)) {
            return CommandEffect::SystemChange;
        }
    }
    if is_read_only_glab_command(&normalized) {
        return CommandEffect::ReadOnlyNetwork;
    }
    if invokes_program(&lower, &["curl", "wget", "ssh", "scp", "nc", "ncat"])
        || lower.contains("glab api ")
    {
        return CommandEffect::NetworkEgress;
    }
    if has_mutating_shell_syntax(&lower) {
        return CommandEffect::WorkspaceMutation;
    }

    let segments = split_shell_segments(&normalized);
    if !segments.is_empty() && segments.iter().all(|segment| is_read_only_segment(segment)) {
        CommandEffect::ReadOnly
    } else {
        CommandEffect::WorkspaceMutation
    }
}

fn strip_safe_redirections(command: &str) -> String {
    command
        .replace("2>/dev/null", "")
        .replace("1>/dev/null", "")
        .replace(">/dev/null", "")
        .replace("2>&1", "")
}

fn is_read_only_glab_command(command: &str) -> bool {
    let segments = split_shell_segments(command);
    segments.iter().any(|segment| segment.starts_with("glab "))
        && segments.iter().all(|segment| is_read_only_segment(segment))
        && !segments
            .iter()
            .any(|segment| matches!(segment.as_str(), "glab --version" | "glab version"))
}

fn invokes_program(command: &str, programs: &[&str]) -> bool {
    command
        .split([';', '|', '&', '(', ')'])
        .filter_map(|segment| segment.split_whitespace().next())
        .any(|program| programs.contains(&program))
}

fn has_mutating_shell_syntax(command: &str) -> bool {
    command.contains('>')
        || command.contains("<<")
        || command.contains("$(")
        || command.contains('`')
        || command.contains(" -delete")
        || command.contains(" -exec")
}

fn split_shell_segments(command: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;

    for character in command.chars() {
        if escaped {
            current.push(character);
            escaped = false;
            continue;
        }
        if character == '\\' && quote != Some('\'') {
            current.push(character);
            escaped = true;
            continue;
        }
        if matches!(character, '\'' | '"') {
            if quote == Some(character) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(character);
            }
            current.push(character);
            continue;
        }
        if quote.is_none() && matches!(character, ';' | '|' | '&') {
            let segment = current.trim();
            if !segment.is_empty() {
                segments.push(segment.to_owned());
            }
            current.clear();
        } else {
            current.push(character);
        }
    }
    let segment = current.trim();
    if !segment.is_empty() {
        segments.push(segment.to_owned());
    }
    segments
}

fn is_read_only_segment(segment: &str) -> bool {
    let words = segment.split_whitespace().collect::<Vec<_>>();
    let Some(program) = words.first().copied() else {
        return false;
    };
    match program {
        "cat" | "ls" | "pwd" | "echo" | "which" | "whoami" | "head" | "tail" | "wc" | "grep"
        | "rg" | "stat" | "realpath" | "readlink" | "uname" | "date" | "id" | "env"
        | "printenv" | "lsb_release" => true,
        // A shell-local directory change has no durable host effect. The
        // remaining pipeline segments are classified independently.
        "cd" => words.len() == 2 && !words[1].starts_with('-'),
        "command" => words.get(1) == Some(&"-v"),
        "find" => !words
            .iter()
            .any(|word| matches!(*word, "-delete" | "-exec" | "-execdir")),
        // Keep xargs fail-closed except for option-free (or null-delimited)
        // dispatch to a command that this classifier already proves read-only.
        "xargs" => {
            let nested = words[1..]
                .iter()
                .copied()
                .skip_while(|word| matches!(*word, "-0" | "--null" | "-r" | "--no-run-if-empty"))
                .collect::<Vec<_>>();
            !nested.is_empty() && is_read_only_segment(&nested.join(" "))
        }
        "git" => matches!(
            words.get(1).copied(),
            Some(
                "status"
                    | "diff"
                    | "log"
                    | "show"
                    | "branch"
                    | "rev-parse"
                    | "remote"
                    | "ls-files"
                    | "ls-tree"
            )
        ),
        "glab" => glab_is_read_only(&words[1..]),
        "systemctl" => systemctl_is_read_only(&words[1..]),
        other => is_help_or_version_probe(other, &words[1..]),
    }
}

fn is_help_or_version_probe(program: &str, args: &[&str]) -> bool {
    if program.contains('=') || args.is_empty() {
        return false;
    }
    if args.iter().any(|arg| {
        matches!(
            *arg,
            "start"
                | "stop"
                | "restart"
                | "enable"
                | "disable"
                | "install"
                | "deploy"
                | "remove"
                | "delete"
        )
    }) {
        return false;
    }
    matches!(args, ["--version" | "-V"])
        || matches!(args.last(), Some(&("--help" | "-h")))
            && args[..args.len() - 1]
                .iter()
                .all(|arg| !arg.starts_with('-'))
}

fn systemctl_is_read_only(args: &[&str]) -> bool {
    let action = args.iter().copied().find(|arg| !arg.starts_with('-'));
    matches!(
        action,
        Some(
            "status"
                | "show"
                | "is-active"
                | "is-enabled"
                | "list-units"
                | "list-unit-files"
                | "show-environment"
        )
    )
}

fn glab_is_read_only(args: &[&str]) -> bool {
    match args {
        ["version", ..] | ["--version", ..] | ["help", ..] => true,
        [group, action, ..]
            if matches!(
                *group,
                "mr" | "issue"
                    | "repo"
                    | "ci"
                    | "release"
                    | "variable"
                    | "ssh-key"
                    | "alias"
                    | "label"
                    | "milestone"
                    | "epic"
                    | "snippet"
                    | "user"
            ) && matches!(*action, "list" | "view" | "trace" | "status") =>
        {
            true
        }
        ["api", rest @ ..] => glab_api_is_get(rest),
        _ => false,
    }
}

fn glab_api_is_get(args: &[&str]) -> bool {
    let mut index = 0;
    while index < args.len() {
        let argument = args[index];
        let inline_method = argument
            .strip_prefix("--method=")
            .or_else(|| argument.strip_prefix("-X"));
        if let Some(method) = inline_method {
            if !method.eq_ignore_ascii_case("GET") {
                return false;
            }
        } else if matches!(argument, "--method" | "-X") {
            let Some(method) = args.get(index + 1) else {
                return false;
            };
            if !method.eq_ignore_ascii_case("GET") {
                return false;
            }
            index += 1;
        }
        index += 1;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observed_glab_probe_is_read_only() {
        assert_eq!(
            classify_command("which glab && glab --version"),
            CommandEffect::ReadOnly
        );
        assert_eq!(
            classify_command("which glab 2>/dev/null && glab --version"),
            CommandEffect::ReadOnly
        );
        assert_eq!(
            classify_command("glab mr view 22 --repo highlydynamic/leju_head_lab"),
            CommandEffect::ReadOnlyNetwork
        );
        assert_eq!(
            classify_command("glab api --method GET projects/1"),
            CommandEffect::ReadOnlyNetwork
        );
    }

    #[test]
    fn install_network_and_mutation_fail_closed() {
        assert_eq!(
            classify_command("sudo apt-get install -y glab"),
            CommandEffect::SystemChange
        );
        assert_eq!(
            classify_command("curl -fsSL https://example.invalid/a -o /tmp/a"),
            CommandEffect::NetworkEgress
        );
        assert_eq!(
            classify_command("glab api -X POST projects/1/issues"),
            CommandEffect::NetworkEgress
        );
        assert_eq!(
            classify_command("glab api --method=DELETE projects/1"),
            CommandEffect::NetworkEgress
        );
        assert_eq!(
            classify_command("which snap; which apt-get; which curl"),
            CommandEffect::ReadOnly
        );
        assert_eq!(
            classify_command(
                "cat /etc/os-release 2>/dev/null || lsb_release -a 2>/dev/null || uname -a"
            ),
            CommandEffect::ReadOnly
        );
        assert_eq!(classify_command("echo rm"), CommandEffect::ReadOnly);
        assert_eq!(
            classify_command("printf x > file"),
            CommandEffect::WorkspaceMutation
        );
        assert_eq!(classify_command("rm -rf ."), CommandEffect::Destructive);
    }

    #[test]
    fn bounded_repository_statistics_are_read_only() {
        assert_eq!(
            classify_command(
                "cd /workspace && find crates -name '*.rs' -type f | xargs wc -l | tail -20"
            ),
            CommandEffect::ReadOnly
        );
        assert_eq!(
            classify_command("find crates -print0 | xargs -0 -r wc -l"),
            CommandEffect::ReadOnly
        );
        assert_eq!(
            classify_command(
                "find crates -name '*.rs' | xargs grep -l '#\\[tokio::test\\]\\|#\\[test\\]' 2>/dev/null | wc -l"
            ),
            CommandEffect::ReadOnly
        );
        assert_eq!(
            classify_command("find crates -print0 | xargs -0 sh -c 'rm \"$1\"'"),
            CommandEffect::WorkspaceMutation
        );
    }

    #[test]
    fn diagnostic_help_and_systemctl_reads_are_read_only() {
        for command in [
            "/usr/bin/aletheon --help",
            "/usr/bin/aletheon memory --help",
            "/usr/bin/aletheon memory-agent -h",
            "systemctl --user status aletheon.service",
            "systemctl show aletheon-core.service -p ActiveState",
            "systemctl list-units --type=service",
        ] {
            assert_eq!(
                classify_command(command),
                CommandEffect::ReadOnly,
                "{command}"
            );
        }
    }

    #[test]
    fn systemctl_mutations_and_compound_help_fail_closed() {
        for command in [
            "systemctl restart aletheon.service",
            "systemctl --user enable --now aletheon.socket",
            "/usr/bin/aletheon --help; touch changed",
            "/usr/bin/aletheon --help | sh",
        ] {
            assert_ne!(
                classify_command(command),
                CommandEffect::ReadOnly,
                "{command}"
            );
        }
    }
}
