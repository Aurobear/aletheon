//! Centralized path constants for Aletheon runtime.

use std::path::PathBuf;

/// User config directory: ~/.aletheon/
pub fn config_dir() -> PathBuf {
    home_dir().join(".aletheon")
}

/// System socket directory (for systemd service units only).
pub const SYSTEM_SOCKET_DIR: &str = "/var/run/aletheon";

/// Backward-compatible alias.
#[deprecated(note = "Use SYSTEM_SOCKET_DIR or user_socket_dir() instead")]
pub const SOCKET_DIR: &str = SYSTEM_SOCKET_DIR;

/// System snapshot directory: /var/lib/aletheon/snapshots
pub const SNAPSHOT_DIR: &str = "/var/lib/aletheon/snapshots";

/// System hooks directory: /etc/aletheon/hooks
pub const HOOKS_SYSTEM_DIR: &str = "/etc/aletheon/hooks";

/// Cgroup prefix for sandbox isolation
pub const CGROUP_PREFIX: &str = "aletheon";

/// XDG config: ~/.config/aletheon/
pub fn xdg_config_dir() -> PathBuf {
    home_dir().join(".config").join("aletheon")
}

/// XDG data: ~/.local/share/aletheon/
pub fn xdg_data_dir() -> PathBuf {
    home_dir().join(".local").join("share").join("aletheon")
}

/// User hooks directory: ~/.aletheon/hooks/
pub fn user_hooks_dir() -> PathBuf {
    config_dir().join("hooks")
}

/// Local hooks directory: .aletheon/hooks/
pub fn local_hooks_dir() -> PathBuf {
    PathBuf::from(".aletheon").join("hooks")
}

/// Skills directory: ~/.aletheon/skills/
pub fn skills_dir() -> PathBuf {
    config_dir().join("skills")
}

/// MCP tokens path: ~/.config/aletheon/mcp_tokens.json
pub fn mcp_tokens_path() -> PathBuf {
    xdg_config_dir().join("mcp_tokens.json")
}

/// Config file path: ~/.aletheon/config.toml
pub fn config_file() -> PathBuf {
    config_dir().join("config.toml")
}

/// Env file path: ~/.aletheon/.env
pub fn env_file() -> PathBuf {
    config_dir().join(".env")
}

/// User-space socket directory: `$XDG_RUNTIME_DIR/aletheon` or `~/.aletheon/`.
pub fn user_socket_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
        PathBuf::from(xdg).join("aletheon")
    } else {
        config_dir()
    }
}

/// Default socket path for user-mode daemon.
pub fn default_socket_path() -> PathBuf {
    user_socket_dir().join("aletheon.sock")
}

fn home_dir() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_socket_path_not_system() {
        let path = default_socket_path();
        let path_str = path.to_string_lossy();
        // Should NOT be the old system path /var/run/aletheon or /run/aletheon
        // (but /run/user/<uid> from XDG_RUNTIME_DIR is fine)
        assert!(!path_str.starts_with("/run/aletheon"), "socket path should not be in /run/aletheon: {}", path_str);
        assert!(!path_str.starts_with("/var/run/"), "socket path should not be in /var/run/: {}", path_str);
        assert!(path_str.ends_with("aletheon.sock"), "should end with aletheon.sock: {}", path_str);
    }

    #[test]
    fn test_default_socket_path_has_parent() {
        let path = default_socket_path();
        assert!(path.parent().is_some(), "socket path should have a parent directory");
    }

    #[test]
    fn test_system_socket_dir_still_available() {
        assert_eq!(SYSTEM_SOCKET_DIR, "/var/run/aletheon");
    }
}
