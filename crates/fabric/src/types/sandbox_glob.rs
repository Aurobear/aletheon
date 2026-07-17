//! Fail-closed deny-glob expansion for sandbox profiles (S1 T8).
//!
//! [`resolve_profile`](crate::resolve_profile) carries deny globs verbatim into
//! [`ResolvedSandboxPolicy::deny_globs`](crate::ResolvedSandboxPolicy). Before a
//! backend can enforce them as concrete bind-over / Landlock rules, the daemon
//! expands each glob against the real filesystem under the sandbox roots.
//!
//! The expansion is **fail-closed**: any cap breach returns
//! [`ProfileResolveError::GlobOverflow`] so the caller denies execution rather
//! than silently under-denying a path that should have been blocked. The three
//! caps mirror the Grok sandbox limits:
//!
//! - `> DENY_GLOB_MAX_ENTRIES` input patterns → overflow (bounded deny list).
//! - a directory tree deeper than `DENY_GLOB_MAX_DEPTH` while entries remain →
//!   overflow (we cannot guarantee complete expansion within budget).
//! - `> DENY_GLOB_MAX_MATCHES` accumulated matches → overflow.
//!
//! Non-existent roots are skipped (exact denies are handled separately via
//! `deny_exact`); symlinked directories are not descended (no escape, no loop).

use std::path::{Path, PathBuf};

use crate::types::sandbox::{
    ProfileResolveError, DENY_GLOB_MAX_DEPTH, DENY_GLOB_MAX_ENTRIES, DENY_GLOB_MAX_MATCHES,
};

/// Expand deny globs into concrete existing paths under `roots`, applying the
/// crate-level fail-closed caps. Returns deduplicated, sorted matches.
pub fn expand_deny_globs(
    globs: &[String],
    roots: &[PathBuf],
) -> Result<Vec<PathBuf>, ProfileResolveError> {
    expand_with_caps(
        globs,
        roots,
        DENY_GLOB_MAX_ENTRIES,
        DENY_GLOB_MAX_DEPTH,
        DENY_GLOB_MAX_MATCHES,
    )
}

/// Cap-parameterized core so the fail-closed branches are unit-testable without
/// materializing thousands of files. `expand_deny_globs` supplies the consts.
fn expand_with_caps(
    globs: &[String],
    roots: &[PathBuf],
    max_entries: usize,
    max_depth: usize,
    max_matches: usize,
) -> Result<Vec<PathBuf>, ProfileResolveError> {
    if globs.len() > max_entries {
        return Err(ProfileResolveError::GlobOverflow);
    }
    // Only real glob patterns are expanded here; exact entries are carried by
    // `deny_exact`. A glob-free list therefore does no filesystem work.
    let patterns: Vec<&str> = globs
        .iter()
        .filter(|g| is_glob(g))
        .map(String::as_str)
        .collect();
    if patterns.is_empty() {
        return Ok(Vec::new());
    }
    let mut matches: Vec<PathBuf> = Vec::new();
    for root in roots {
        if !root.is_dir() {
            continue;
        }
        walk(
            root,
            root,
            0,
            &patterns,
            max_depth,
            max_matches,
            &mut matches,
        )?;
    }
    matches.sort();
    matches.dedup();
    Ok(matches)
}

/// Recursive, depth-bounded walk. Every entry is tested against every pattern;
/// exceeding depth or match budget fails closed.
fn walk(
    root: &Path,
    dir: &Path,
    depth: usize,
    patterns: &[&str],
    max_depth: usize,
    max_matches: usize,
    matches: &mut Vec<PathBuf>,
) -> Result<(), ProfileResolveError> {
    if depth > max_depth {
        // Tree is deeper than the expansion budget: we cannot prove the deny
        // set is complete, so refuse rather than under-deny.
        return Err(ProfileResolveError::GlobOverflow);
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        // An unreadable directory cannot be expanded; skip it (the backend still
        // applies root-level restrictions). Do not fail the whole resolution.
        Err(_) => return Ok(()),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        if let Ok(rel) = path.strip_prefix(root) {
            if patterns.iter().any(|pat| glob_match(pat, rel)) {
                matches.push(path.clone());
                if matches.len() > max_matches {
                    return Err(ProfileResolveError::GlobOverflow);
                }
            }
        }
        if file_type.is_dir() {
            // Do not follow symlinked directories: no escape out of the root,
            // no cycle. `file_type` reflects the entry itself, not its target.
            walk(
                root,
                &path,
                depth + 1,
                patterns,
                max_depth,
                max_matches,
                matches,
            )?;
        }
    }
    Ok(())
}

/// True when `entry` contains a glob metacharacter (`*`, `?`). Exact paths are
/// handled by `deny_exact`, not here.
fn is_glob(entry: &str) -> bool {
    entry.contains('*') || entry.contains('?')
}

/// Match a `/`-separated glob pattern against a relative path. `**` matches zero
/// or more path segments; `*`/`?` match within a single segment.
fn glob_match(pattern: &str, path: &Path) -> bool {
    let pat: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
    let segs: Vec<String> = path
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    match_segments(&pat, &segs)
}

fn match_segments(pat: &[&str], segs: &[String]) -> bool {
    match pat.split_first() {
        None => segs.is_empty(),
        Some((&"**", rest)) => {
            // `**` consumes zero or more leading segments.
            (0..=segs.len()).any(|i| match_segments(rest, &segs[i..]))
        }
        Some((seg, rest)) => {
            !segs.is_empty() && segment_match(seg, &segs[0]) && match_segments(rest, &segs[1..])
        }
    }
}

/// Wildcard match within one path segment (`*` = any run, `?` = one char).
fn segment_match(pat: &str, name: &str) -> bool {
    let p: Vec<char> = pat.chars().collect();
    let n: Vec<char> = name.chars().collect();
    wildcard(&p, &n)
}

fn wildcard(pat: &[char], name: &[char]) -> bool {
    match pat.split_first() {
        None => name.is_empty(),
        Some(('*', rest)) => (0..=name.len()).any(|i| wildcard(rest, &name[i..])),
        Some(('?', rest)) => !name.is_empty() && wildcard(rest, &name[1..]),
        Some((c, rest)) => !name.is_empty() && name[0] == *c && wildcard(rest, &name[1..]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn touch(path: &Path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, b"x").unwrap();
    }

    #[test]
    fn recursive_star_matches_nested_extension_only() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        touch(&root.join("a/b/secret.pem"));
        touch(&root.join("a/notes.txt"));
        touch(&root.join("top.pem"));

        let out = expand_deny_globs(&["**/*.pem".to_string()], &[root.clone()]).unwrap();
        assert!(out.contains(&root.join("a/b/secret.pem")));
        assert!(out.contains(&root.join("top.pem")));
        assert!(!out.iter().any(|p| p.ends_with("notes.txt")));
    }

    #[test]
    fn matches_dotfile_by_name_at_any_depth() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        touch(&root.join("svc/config/.env"));
        touch(&root.join("svc/config/app.yaml"));

        let out = expand_deny_globs(&["**/.env".to_string()], &[root.clone()]).unwrap();
        assert_eq!(out, vec![root.join("svc/config/.env")]);
    }

    #[test]
    fn exact_entries_are_ignored_here() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        touch(&root.join("plain.txt"));
        // No glob metacharacter → not expanded (handled by deny_exact).
        let out = expand_deny_globs(&["/etc/shadow".to_string()], &[root]).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn missing_root_is_skipped_not_an_error() {
        let out = expand_deny_globs(
            &["**/*.pem".to_string()],
            &[PathBuf::from("/nonexistent-sandbox-root-xyz")],
        )
        .unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn too_many_patterns_fails_closed() {
        let globs: Vec<String> = (0..(DENY_GLOB_MAX_ENTRIES + 1))
            .map(|i| format!("**/*.x{i}"))
            .collect();
        let err = expand_deny_globs(&globs, &[]).unwrap_err();
        assert!(matches!(err, ProfileResolveError::GlobOverflow));
    }

    #[test]
    fn depth_beyond_cap_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        // Nested three levels; a max_depth of 1 cannot cover it.
        touch(&root.join("l1/l2/l3/deep.pem"));
        let err = expand_with_caps(&["**/*.pem".to_string()], &[root], 8, 1, 4096).unwrap_err();
        assert!(matches!(err, ProfileResolveError::GlobOverflow));
    }

    #[test]
    fn matches_beyond_cap_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        for i in 0..3 {
            touch(&root.join(format!("k{i}.pem")));
        }
        // Match budget of 2 with 3 matching files → overflow (fail-closed).
        let err = expand_with_caps(&["**/*.pem".to_string()], &[root], 8, 8, 2).unwrap_err();
        assert!(matches!(err, ProfileResolveError::GlobOverflow));
    }

    #[test]
    fn within_caps_returns_all_matches() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        for i in 0..3 {
            touch(&root.join(format!("k{i}.pem")));
        }
        let out = expand_with_caps(&["**/*.pem".to_string()], &[root], 8, 8, 4096).unwrap();
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn segment_wildcards_do_not_cross_separators() {
        // `*` matches within a segment only; it must not span `/`.
        assert!(segment_match("*.pem", "secret.pem"));
        assert!(!segment_match("*.pem", "secret.key"));
        assert!(segment_match("id_?sa", "id_rsa"));
        assert!(!glob_match("*.pem", Path::new("a/b.pem")));
        assert!(glob_match("a/*.pem", Path::new("a/b.pem")));
    }
}
