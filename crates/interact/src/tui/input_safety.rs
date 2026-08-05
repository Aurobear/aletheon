//! Fail-closed terminal input and workspace attachment validation.

use std::path::{Component, Path, PathBuf};

use fabric::WorkspacePolicy;

pub const MAX_PASTE_BYTES: usize = 1_000_000;
pub const MAX_ATTACHMENT_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentReference {
    pub requested: String,
    pub resolved: PathBuf,
    pub byte_len: u64,
}

/// Preserve editor whitespace while dropping ANSI/OSC and C0/C1 controls.
pub fn sanitize_paste(input: &str) -> String {
    let mut output = String::with_capacity(input.len().min(MAX_PASTE_BYTES));
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            match chars.peek().copied() {
                Some(']') => {
                    chars.next();
                    while let Some(next) = chars.next() {
                        if next == '\u{7}' {
                            break;
                        }
                        if next == '\u{1b}' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                Some('P' | '^' | '_' | 'X') => {
                    chars.next();
                    while let Some(next) = chars.next() {
                        if next == '\u{1b}' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                Some(_) => {
                    if chars.next() == Some('[') {
                        for final_byte in chars.by_ref() {
                            if ('@'..='~').contains(&final_byte) {
                                break;
                            }
                        }
                    } else {
                        let _ = chars.next();
                    }
                }
                None => {}
            }
            continue;
        }
        let code = ch as u32;
        if (0x00..=0x08).contains(&code)
            || (0x0B..=0x1F).contains(&code)
            || (0x7F..=0x9F).contains(&code)
        {
            continue;
        }
        if output.len() + ch.len_utf8() > MAX_PASTE_BYTES {
            break;
        }
        output.push(ch);
    }
    output
}

/// Resolve an `@path` only when it is a regular, non-symlink file inside the
/// effective workspace authority and below the attachment size limit.
pub fn resolve_attachment(
    reference: &str,
    workspace: &WorkspacePolicy,
) -> Result<AttachmentReference, String> {
    let raw = reference
        .strip_prefix('@')
        .ok_or_else(|| "attachment must start with @".to_string())?
        .trim();
    if raw.is_empty() || raw.chars().any(|ch| ch.is_control()) {
        return Err("attachment path is empty or contains control characters".into());
    }
    let requested = Path::new(raw);
    if requested
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err("attachment path traversal is not allowed".into());
    }
    let candidate = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        workspace.cwd().join(requested)
    };
    let metadata = std::fs::symlink_metadata(&candidate)
        .map_err(|error| format!("attachment cannot be read: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        return Err("attachment must be a regular non-symlink file".into());
    }
    let resolved = std::fs::canonicalize(&candidate)
        .map_err(|error| format!("attachment cannot be resolved: {error}"))?;
    if !workspace
        .writable_roots()
        .iter()
        .any(|root| resolved.starts_with(root))
    {
        return Err("attachment is outside workspace authority".into());
    }
    if workspace
        .protected_paths()
        .credential_paths()
        .iter()
        .any(|protected| resolved.starts_with(protected))
    {
        return Err("attachment is protected by workspace policy".into());
    }
    let byte_len = metadata.len();
    if byte_len > MAX_ATTACHMENT_BYTES {
        return Err(format!("attachment exceeds {MAX_ATTACHMENT_BYTES} bytes"));
    }
    Ok(AttachmentReference {
        requested: raw.to_owned(),
        resolved,
        byte_len,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn paste_sanitizer_removes_ansi_osc_and_controls() {
        assert_eq!(
            sanitize_paste("ok\x1b[31m red\x1b[0m\x1b]0;title\x07\nnext"),
            "ok red\nnext"
        );
    }

    #[test]
    fn attachment_resolution_rejects_traversal_and_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        fs::write(root.join("ok.txt"), "ok").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(root.join("ok.txt"), root.join("link.txt")).unwrap();
        let workspace = WorkspacePolicy::from_resolved_roots(root, vec![]).unwrap();
        assert!(resolve_attachment("@../ok.txt", &workspace).is_err());
        assert_eq!(
            resolve_attachment("@ok.txt", &workspace).unwrap().byte_len,
            2
        );
        #[cfg(unix)]
        assert!(resolve_attachment("@link.txt", &workspace).is_err());
    }
}
