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
        include_str!("../../../../../../agents/general-agent.md"),
    ),
    (
        "orchestrator-agent.md",
        include_str!("../../../../../../agents/orchestrator-agent.md"),
    ),
    (
        "safe-agent.md",
        include_str!("../../../../../../agents/safe-agent.md"),
    ),
    (
        "code-agent.md",
        include_str!("../../../../../../agents/code-agent.md"),
    ),
    (
        "fs-agent.md",
        include_str!("../../../../../../agents/fs-agent.md"),
    ),
    (
        "net-agent.md",
        include_str!("../../../../../../agents/net-agent.md"),
    ),
    (
        "admin-agent.md",
        include_str!("../../../../../../agents/admin-agent.md"),
    ),
    (
        "robot-agent.md",
        include_str!("../../../../../../agents/robot-agent.md"),
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
