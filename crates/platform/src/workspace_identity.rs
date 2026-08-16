//! Canonical host-derived workspace identity shared by workspace subsystems.

/// Canonical workspace identity, resisting path-alias and symlink bypass.
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const MAX_GIT_CONFIG_BYTES: u64 = 1024 * 1024;

pub fn workspace_identity(
    canonical_path: &Path,
) -> application::workspace_identity::WorkspaceIdentity {
    application::workspace_identity::WorkspaceIdentity {
        canonical_path: canonical_path.to_path_buf(),
        repo_fingerprint: repository_fingerprint(canonical_path),
    }
}

/// Broad roots cannot receive durable trust. Besides the user's home, any
/// filesystem root is broad by construction.
pub fn is_broad_unrecordable_root(canonical_path: &Path) -> bool {
    canonical_path.parent().is_none()
        || dirs::home_dir().is_some_and(|home| {
            let home = std::fs::canonicalize(&home).unwrap_or(home);
            canonical_path == home
        })
}

fn repository_fingerprint(workspace: &Path) -> Option<String> {
    let config = git_config_path(workspace)?;
    let metadata = std::fs::symlink_metadata(&config).ok()?;
    if !metadata.file_type().is_file() || metadata.len() > MAX_GIT_CONFIG_BYTES {
        return None;
    }
    let content = std::fs::read_to_string(config).ok()?;
    let mut remotes = parse_remote_urls(&content)
        .into_iter()
        .filter_map(|remote| normalize_remote_url(&remote))
        .collect::<Vec<_>>();
    remotes.sort();
    remotes.dedup();
    if remotes.is_empty() {
        return None;
    }
    let mut hasher = Sha256::new();
    for remote in remotes {
        hasher.update(remote.as_bytes());
        hasher.update([0]);
    }
    Some(format!("sha256:{:x}", hasher.finalize()))
}

fn git_config_path(workspace: &Path) -> Option<PathBuf> {
    for ancestor in workspace.ancestors() {
        let dot_git = ancestor.join(".git");
        let Ok(metadata) = std::fs::symlink_metadata(&dot_git) else {
            continue;
        };
        if metadata.file_type().is_symlink() {
            return None;
        }
        if metadata.is_dir() {
            return Some(dot_git.join("config"));
        }
        if metadata.is_file() && metadata.len() <= 4096 {
            let pointer = std::fs::read_to_string(&dot_git).ok()?;
            let git_dir = pointer.trim().strip_prefix("gitdir:")?.trim();
            let git_dir = if Path::new(git_dir).is_absolute() {
                PathBuf::from(git_dir)
            } else {
                ancestor.join(git_dir)
            };
            let common_dir_file = git_dir.join("commondir");
            if std::fs::metadata(&common_dir_file).is_ok_and(|metadata| metadata.len() <= 4096) {
                if let Ok(common_dir) = std::fs::read_to_string(common_dir_file) {
                    let common_dir = common_dir.trim();
                    let common_dir = if Path::new(common_dir).is_absolute() {
                        PathBuf::from(common_dir)
                    } else {
                        git_dir.join(common_dir)
                    };
                    return Some(common_dir.join("config"));
                }
            }
            return Some(git_dir.join("config"));
        }
    }
    None
}

fn parse_remote_urls(config: &str) -> Vec<String> {
    let mut in_remote = false;
    let mut urls = Vec::new();
    for raw in config.lines() {
        let line = raw.trim();
        if line.starts_with('[') {
            in_remote = line
                .strip_prefix('[')
                .and_then(|line| line.strip_suffix(']'))
                .is_some_and(|section| {
                    section
                        .trim_start()
                        .to_ascii_lowercase()
                        .starts_with("remote \"")
                });
            continue;
        }
        if !in_remote || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            if key.trim().eq_ignore_ascii_case("url") {
                let value = value.trim();
                if !value.is_empty() {
                    urls.push(value.to_string());
                }
            }
        }
    }
    urls
}

fn normalize_remote_url(remote: &str) -> Option<String> {
    let remote = remote.trim();
    if let Ok(mut url) = url::Url::parse(remote) {
        if !matches!(url.scheme(), "http" | "https" | "ssh" | "git" | "file") {
            return None;
        }
        let _ = url.set_username("");
        let _ = url.set_password(None);
        url.set_query(None);
        url.set_fragment(None);
        let path = url.path().trim_end_matches('/').trim_end_matches(".git");
        let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
        let port = url
            .port()
            .map(|port| format!(":{port}"))
            .unwrap_or_default();
        return Some(format!(
            "{}://{host}{port}{path}",
            url.scheme().to_ascii_lowercase()
        ));
    }
    // Git's SCP-like syntax: [user@]host:path. Drop the user and normalize
    // only host/path; this parser never shells out to git.
    let (authority, path) = remote.split_once(':')?;
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    if host.is_empty() || path.is_empty() || host.contains('/') || host.contains('\\') {
        return None;
    }
    Some(format!(
        "ssh://{}/{}",
        host.to_ascii_lowercase(),
        path.trim_start_matches('/')
            .trim_end_matches('/')
            .trim_end_matches(".git")
    ))
}
