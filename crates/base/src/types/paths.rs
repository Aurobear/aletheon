//! Centralized path constants for Aletheon runtime.

use std::path::PathBuf;

/// User config directory: ~/.aletheon/
pub fn config_dir() -> PathBuf {
    home_dir().join(".aletheon")
}

/// System socket directory: /var/run/aletheon/
pub const SOCKET_DIR: &str = "/var/run/aletheon";

/// Daemon socket path: /var/run/aletheon/aletheon.sock (symlinked as /run/aletheon/aletheon.sock)
pub fn socket_path() -> PathBuf {
    PathBuf::from(SOCKET_DIR).join("aletheon.sock")
}

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

/// Agents directory: ~/.aletheon/agents/
pub fn agents_dir() -> PathBuf {
    config_dir().join("agents")
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

fn home_dir() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
}
