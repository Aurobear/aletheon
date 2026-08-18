//! Shipped agent profiles bundled into the binary and seeded into the runtime
//! agents directory on startup.
//!
//! `deploy`/`install` do not sync the `agents/` directory, and the runtime
//! resolves its data dir differently across system/user modes, so a fresh or
//! updated deployment could be missing the shipped profiles (e.g. the capable
//! `general-agent` default). Seeding from the binary removes that drift: the
//! daemon always provisions the current shipped profiles into whatever data dir
//! it actually uses. User-authored profiles (any file not listed here) are left
//! untouched; shipped profiles are rewritten only when their content differs.

use std::path::Path;

/// (filename, bundled content) for every profile the project ships.
const PROFILES: &[(&str, &str)] = &[
    (
        "general-agent.md",
        include_str!("../../../../../agents/general-agent.md"),
    ),
    (
        "orchestrator-agent.md",
        include_str!("../../../../../agents/orchestrator-agent.md"),
    ),
    (
        "safe-agent.md",
        include_str!("../../../../../agents/safe-agent.md"),
    ),
    (
        "safe-agent.toml",
        include_str!("../../../../../agents/safe-agent.toml"),
    ),
    (
        "code-agent.md",
        include_str!("../../../../../agents/code-agent.md"),
    ),
    (
        "code-agent.toml",
        include_str!("../../../../../agents/code-agent.toml"),
    ),
    (
        "fs-agent.md",
        include_str!("../../../../../agents/fs-agent.md"),
    ),
    (
        "fs-agent.toml",
        include_str!("../../../../../agents/fs-agent.toml"),
    ),
    (
        "net-agent.md",
        include_str!("../../../../../agents/net-agent.md"),
    ),
    (
        "net-agent.toml",
        include_str!("../../../../../agents/net-agent.toml"),
    ),
    (
        "admin-agent.md",
        include_str!("../../../../../agents/admin-agent.md"),
    ),
    (
        "admin-agent.toml",
        include_str!("../../../../../agents/admin-agent.toml"),
    ),
    (
        "robot-agent.md",
        include_str!("../../../../../agents/robot-agent.md"),
    ),
    (
        "planner-agent.md",
        include_str!("../../../../../agents/planner-agent.md"),
    ),
    (
        "explorer-agent.md",
        include_str!("../../../../../agents/explorer-agent.md"),
    ),
    (
        "executor-agent.md",
        include_str!("../../../../../agents/executor-agent.md"),
    ),
    (
        "tester-agent.md",
        include_str!("../../../../../agents/tester-agent.md"),
    ),
    (
        "reviewer-agent.md",
        include_str!("../../../../../agents/reviewer-agent.md"),
    ),
    (
        "fixer-agent.md",
        include_str!("../../../../../agents/fixer-agent.md"),
    ),
];

/// Write the bundled shipped profiles into `agents_dir`, creating it if needed.
/// Shipped profiles are overwritten to stay current with the release; any other
/// file in the directory (a user-authored profile) is left untouched. Failures
/// are logged and never fatal — the daemon still starts with whatever profiles
/// already exist on disk.
pub(super) fn seed(agents_dir: &Path) {
    if let Err(error) = std::fs::create_dir_all(agents_dir) {
        tracing::warn!(
            %error,
            dir = %agents_dir.display(),
            "could not create agents dir for profile seeding"
        );
        return;
    }
    let mut written = 0usize;
    for (name, content) in PROFILES {
        let path = agents_dir.join(name);
        // Skip the write when the on-disk content already matches, to avoid
        // needless churn and preserve mtime for unchanged profiles.
        if std::fs::read_to_string(&path).ok().as_deref() == Some(*content) {
            continue;
        }
        match std::fs::write(&path, content) {
            Ok(()) => written += 1,
            Err(error) => {
                tracing::warn!(%error, profile = name, "could not seed shipped agent profile")
            }
        }
    }
    if written > 0 {
        tracing::info!(count = written, "seeded shipped agent profiles");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn specialized_role_profiles_are_bundled() {
        let expected = [
            "planner-agent",
            "explorer-agent",
            "executor-agent",
            "tester-agent",
            "reviewer-agent",
            "fixer-agent",
        ];

        for id in expected {
            let filename = format!("{id}.md");
            let (_, markdown) = PROFILES
                .iter()
                .find(|(name, _)| *name == filename)
                .unwrap_or_else(|| panic!("missing bundled profile {id}"));
            assert!(markdown.contains(&format!("name: {id}")));
        }
    }

    #[test]
    fn default_code_profile_can_call_and_delegate_agent_controls() {
        let (_, markdown) = PROFILES
            .iter()
            .find(|(name, _)| *name == "code-agent.md")
            .expect("bundled code-agent Markdown");
        for control in ["agent_spawn", "agent_wait", "agent_cancel", "agent_list"] {
            assert!(
                markdown.contains(control),
                "code-agent is missing {control}"
            );
        }
        assert!(markdown.contains("delegate_tools: [\"*\"]"));
    }

    #[test]
    fn shipped_profiles_do_not_claim_host_only_settlement_tools() {
        for (name, profile) in PROFILES {
            for host_only_tool in ["change_accept", "change_rollback"] {
                assert!(
                    !profile.contains(host_only_tool),
                    "shipped profile {name} exposes Host-only tool {host_only_tool}"
                );
            }
        }
    }

    #[test]
    fn seed_refreshes_shipped_legacy_mirrors_without_touching_user_profiles() {
        let temporary = tempfile::tempdir().unwrap();
        let agents_dir = temporary.path().join("agents");
        std::fs::create_dir_all(&agents_dir).unwrap();
        std::fs::write(agents_dir.join("fs-agent.toml"), "stale").unwrap();
        std::fs::write(agents_dir.join("custom-agent.md"), "custom").unwrap();

        seed(&agents_dir);

        assert_eq!(
            std::fs::read_to_string(agents_dir.join("fs-agent.toml")).unwrap(),
            include_str!("../../../../../agents/fs-agent.toml")
        );
        assert_eq!(
            std::fs::read_to_string(agents_dir.join("custom-agent.md")).unwrap(),
            "custom"
        );
    }
}
