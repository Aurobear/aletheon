use std::path::{Path, PathBuf};

use tracing::{debug, warn};

/// Default maximum number of results returned by a single scan.
const DEFAULT_MAX_RESULTS: usize = 8192;

/// Glob scanner that discovers files matching a set of glob patterns.
///
/// Prefers `rg --files --hidden --no-ignore` for speed, falling back to a
/// pure-Rust `walkdir` traversal when `rg` is not available.
pub struct GlobScanner {
    /// Maximum number of paths to return before truncating.
    pub max_results: usize,
}

impl Default for GlobScanner {
    fn default() -> Self {
        Self {
            max_results: DEFAULT_MAX_RESULTS,
        }
    }
}

impl GlobScanner {
    /// Create a scanner with a custom result cap.
    pub fn new(max_results: usize) -> Self {
        Self { max_results }
    }

    /// Scan `base_dir` for files matching any of the given glob patterns.
    ///
    /// Returns at most [`self.max_results`] paths.  Paths are returned in the
    /// order they are discovered.
    pub fn scan(&self, globs: &[String], base_dir: &Path) -> Vec<PathBuf> {
        if globs.is_empty() {
            return Vec::new();
        }

        // Try ripgrep first for speed.
        if let Some(results) = self.try_rg(globs, base_dir) {
            return results;
        }

        // Fallback: walkdir + glob matching.
        self.walkdir_scan(globs, base_dir)
    }

    // ---- ripgrep path ---------------------------------------------------

    fn try_rg(&self, globs: &[String], base_dir: &Path) -> Option<Vec<PathBuf>> {
        let rg_path = which::which("rg").ok()?;

        let mut cmd = std::process::Command::new(&rg_path);
        cmd.arg("--files")
            .arg("--hidden")
            .arg("--no-ignore")
            .current_dir(base_dir);

        // Add each glob as a --glob filter.
        for glob in globs {
            cmd.arg("--glob").arg(glob);
        }

        debug!(base = %base_dir.display(), globs = ?globs, "Running rg scan");

        let output = match cmd.output() {
            Ok(o) => o,
            Err(e) => {
                warn!(error = %e, "Failed to run rg, falling back to walkdir");
                return None;
            }
        };

        if !output.status.success() {
            // rg returns exit code 1 when no matches — that is fine.
            let code = output.status.code().unwrap_or(-1);
            if code > 1 {
                warn!(code, "rg exited with unexpected code, falling back to walkdir");
                return None;
            }
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut results = Vec::new();

        for line in stdout.lines() {
            if results.len() >= self.max_results {
                break;
            }
            let path = base_dir.join(line);
            results.push(path);
        }

        Some(results)
    }

    // ---- walkdir fallback ------------------------------------------------

    fn walkdir_scan(&self, globs: &[String], base_dir: &Path) -> Vec<PathBuf> {
        use std::fs;

        let compiled: Vec<SimpleGlob> = globs
            .iter()
            .filter_map(|g| match SimpleGlob::new(g) {
                Ok(p) => Some(p),
                Err(e) => {
                    warn!(pattern = g, error = %e, "Invalid glob pattern, skipping");
                    None
                }
            })
            .collect();

        if compiled.is_empty() {
            return Vec::new();
        }

        let mut results = Vec::new();
        let mut stack: Vec<PathBuf> = vec![base_dir.to_path_buf()];

        while let Some(dir) = stack.pop() {
            if results.len() >= self.max_results {
                break;
            }

            let entries = match fs::read_dir(&dir) {
                Ok(e) => e,
                Err(e) => {
                    debug!(dir = %dir.display(), error = %e, "Cannot read directory, skipping");
                    continue;
                }
            };

            for entry in entries {
                if results.len() >= self.max_results {
                    break;
                }

                let entry = match entry {
                    Ok(e) => e,
                    Err(e) => {
                        debug!(error = %e, "Error reading dir entry");
                        continue;
                    }
                };

                let path = entry.path();
                let rel = match path.strip_prefix(base_dir) {
                    Ok(r) => r,
                    Err(_) => continue,
                };

                let rel_str = rel.to_string_lossy();

                let is_match = compiled.iter().any(|pat| pat.matches(&rel_str));
                if is_match {
                    results.push(path.clone());
                }

                // Recurse into directories.
                if path.is_dir() {
                    stack.push(path);
                }
            }
        }

        results
    }
}

// ---- Simple glob matcher (avoids external `glob` crate dependency) ----

/// A simple glob pattern supporting `*` (single segment) and `**` (recursive).
struct SimpleGlob {
    /// The original pattern string, kept for debugging.
    #[allow(dead_code)]
    pattern: String,
    /// Tokenized segments after splitting on `/`.
    segments: Vec<GlobSegment>,
}

#[derive(Debug, Clone)]
enum GlobSegment {
    /// `**` — matches zero or more path segments.
    Recursive,
    /// A literal segment (no wildcards).
    Literal(String),
    /// A segment containing `*` wildcards (stored as a pattern to match
    /// against a single path component).
    Wildcard(String),
}

impl SimpleGlob {
    fn new(pattern: &str) -> Result<Self, String> {
        let segments: Vec<GlobSegment> = pattern
            .split('/')
            .map(|seg| {
                if seg == "**" {
                    GlobSegment::Recursive
                } else if seg.contains('*') {
                    GlobSegment::Wildcard(seg.to_string())
                } else {
                    GlobSegment::Literal(seg.to_string())
                }
            })
            .collect();

        if segments.is_empty() {
            return Err("Empty glob pattern".into());
        }

        Ok(Self {
            pattern: pattern.to_string(),
            segments,
        })
    }

    fn matches(&self, rel_path: &str) -> bool {
        let path_segments: Vec<&str> = rel_path.split('/').collect();
        self.match_segments(&self.segments, &path_segments)
    }

    fn match_segments(&self, pattern: &[GlobSegment], path: &[&str]) -> bool {
        match (pattern.first(), path.first()) {
            // Both exhausted — match.
            (None, None) => true,
            // Pattern exhausted but path remains — no match.
            (None, Some(_)) => false,
            // `**` at the start of pattern.
            (Some(GlobSegment::Recursive), _) => {
                let rest = &pattern[1..];
                // `**` can match zero segments.
                if self.match_segments(rest, path) {
                    return true;
                }
                // Or consume one path segment and try again.
                for i in 1..=path.len() {
                    if self.match_segments(rest, &path[i..]) {
                        return true;
                    }
                }
                false
            }
            // Literal segment.
            (Some(GlobSegment::Literal(lit)), Some(seg)) => {
                if lit == *seg {
                    self.match_segments(&pattern[1..], &path[1..])
                } else {
                    false
                }
            }
            // Wildcard segment — matches a single path component.
            (Some(GlobSegment::Wildcard(pat)), Some(seg)) => {
                if wildcard_match(pat, seg) {
                    self.match_segments(&pattern[1..], &path[1..])
                } else {
                    false
                }
            }
            // Pattern has segments but path is exhausted.
            (Some(_), None) => false,
        }
    }
}

/// Simple single-segment wildcard matching: `*` matches any sequence of
/// characters within a single path component.
fn wildcard_match(pattern: &str, text: &str) -> bool {
    let mut pi = 0;
    let mut ti = 0;
    let mut star_pi = None;
    let mut star_ti = 0;

    let pat_bytes = pattern.as_bytes();
    let text_bytes = text.as_bytes();

    while ti < text_bytes.len() {
        if pi < pat_bytes.len() && (pat_bytes[pi] == b'?' || pat_bytes[pi] == text_bytes[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < pat_bytes.len() && pat_bytes[pi] == b'*' {
            star_pi = Some(pi);
            star_ti = ti;
            pi += 1;
        } else if let Some(sp) = star_pi {
            pi = sp + 1;
            star_ti += 1;
            ti = star_ti;
        } else {
            return false;
        }
    }

    while pi < pat_bytes.len() && pat_bytes[pi] == b'*' {
        pi += 1;
    }

    pi == pat_bytes.len()
}

/// Mask paths by binding `/dev/null` over them.
///
/// Given a list of paths to mask, returns the flattened `bwrap` argument
/// sequence: `["--ro-bind", "/dev/null", path, "--ro-bind", "/dev/null", ...]`.
pub fn mask_paths_args(paths: &[PathBuf]) -> Vec<String> {
    let mut args = Vec::with_capacity(paths.len() * 3);
    for path in paths {
        args.push("--ro-bind".into());
        args.push("/dev/null".into());
        args.push(path.to_string_lossy().to_string());
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_scanner_empty_globs() {
        let scanner = GlobScanner::default();
        let results = scanner.scan(&[], Path::new("/tmp"));
        assert!(results.is_empty());
    }

    #[test]
    fn test_scanner_walkdir_matches_txt_files() {
        // Create a temp directory with some files.
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();

        fs::write(base.join("hello.txt"), "hello").unwrap();
        fs::write(base.join("world.rs"), "fn main() {}").unwrap();
        fs::create_dir(base.join("sub")).unwrap();
        fs::write(base.join("sub/nested.txt"), "nested").unwrap();

        let scanner = GlobScanner::default();
        let results = scanner.scan(&["**/*.txt".to_string()], base);

        assert_eq!(results.len(), 2, "Expected 2 .txt files, got {:?}", results);
        let names: Vec<String> = results
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert!(names.contains(&"hello.txt".to_string()));
        assert!(names.contains(&"nested.txt".to_string()));
    }

    #[test]
    fn test_scanner_max_results_cap() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();

        // Create more files than the cap.
        for i in 0..20 {
            fs::write(base.join(format!("file_{i:03}.txt")), "").unwrap();
        }

        let scanner = GlobScanner::new(5);
        let results = scanner.scan(&["**/*.txt".to_string()], base);

        assert!(
            results.len() <= 5,
            "Expected at most 5 results, got {}",
            results.len()
        );
    }

    #[test]
    fn test_scanner_no_matches() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        fs::write(base.join("main.rs"), "fn main() {}").unwrap();

        let scanner = GlobScanner::default();
        let results = scanner.scan(&["**/*.py".to_string()], base);
        assert!(results.is_empty());
    }

    #[test]
    fn test_scanner_multiple_globs() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();

        fs::write(base.join("secret.env"), "KEY=val").unwrap();
        fs::write(base.join("id_rsa.key"), "-----BEGIN").unwrap();
        fs::write(base.join("readme.md"), "# Hello").unwrap();

        let scanner = GlobScanner::default();
        let results = scanner.scan(
            &["**/*.env".to_string(), "**/*.key".to_string()],
            base,
        );

        assert_eq!(results.len(), 2, "Expected 2 matches, got {:?}", results);
    }

    #[test]
    fn test_mask_paths_args_format() {
        let paths = vec![
            PathBuf::from("/tmp/a.env"),
            PathBuf::from("/tmp/b.key"),
        ];
        let args = mask_paths_args(&paths);
        assert_eq!(args.len(), 6);
        assert_eq!(args[0], "--ro-bind");
        assert_eq!(args[1], "/dev/null");
        assert_eq!(args[2], "/tmp/a.env");
        assert_eq!(args[3], "--ro-bind");
        assert_eq!(args[4], "/dev/null");
        assert_eq!(args[5], "/tmp/b.key");
    }

    #[test]
    fn test_scanner_invalid_glob_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        fs::write(base.join("file.txt"), "data").unwrap();

        let scanner = GlobScanner::default();
        // Invalid pattern + valid pattern — invalid should be skipped.
        let results = scanner.scan(
            &["**/*.txt".to_string()],
            base,
        );
        assert_eq!(results.len(), 1);
    }
}
