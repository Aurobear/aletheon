//! Bounded workspace file discovery for the `@` attachment palette.

use std::{collections::VecDeque, fs, path::PathBuf};

use ::contracts::WorkspacePolicy;

const MAX_VISITED_ENTRIES: usize = 20_000;
const MAX_RESULTS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileCandidate {
    pub relative_path: String,
    pub byte_len: u64,
}

/// Return a bounded, deterministic first page without following symlinks.
pub fn find_files(workspace: &WorkspacePolicy, query: &str) -> Vec<FileCandidate> {
    let root = workspace.cwd();
    let query = query.trim_start_matches('@').to_lowercase();
    let mut queue = VecDeque::from([root.to_path_buf()]);
    let mut visited = 0_usize;
    let mut matches = Vec::new();

    while let Some(directory) = queue.pop_front() {
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        let mut entries = entries.filter_map(Result::ok).collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            visited += 1;
            if visited > MAX_VISITED_ENTRIES {
                break;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if matches!(name.as_ref(), ".git" | "target" | ".cache") {
                continue;
            }
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                queue.push_back(entry.path());
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            let Ok(relative) = entry.path().strip_prefix(root).map(PathBuf::from) else {
                continue;
            };
            let relative_path = relative.to_string_lossy().replace('\\', "/");
            let lower = relative_path.to_lowercase();
            if !query.is_empty() && !fuzzy_subsequence(&lower, &query) {
                continue;
            }
            let byte_len = entry.metadata().map_or(0, |metadata| metadata.len());
            let prefix = lower.starts_with(&query);
            matches.push((!prefix, relative_path, byte_len));
        }
        if visited > MAX_VISITED_ENTRIES {
            break;
        }
    }
    matches.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    matches
        .into_iter()
        .take(MAX_RESULTS)
        .map(|(_, relative_path, byte_len)| FileCandidate {
            relative_path,
            byte_len,
        })
        .collect()
}

fn fuzzy_subsequence(value: &str, query: &str) -> bool {
    let mut value = value.chars();
    query.chars().all(|needle| value.any(|item| item == needle))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn u_input_001_ten_thousand_files_return_a_first_page_within_300ms() {
        let temp = tempfile::tempdir().unwrap();
        for index in 0..10_000 {
            fs::write(temp.path().join(format!("file-{index:05}.rs")), b"x").unwrap();
        }
        let workspace =
            WorkspacePolicy::from_resolved_roots(temp.path().to_path_buf(), vec![]).unwrap();
        let started = Instant::now();
        let results = find_files(&workspace, "@file-099");
        let elapsed = started.elapsed();
        assert!(!results.is_empty());
        assert!(results.len() <= MAX_RESULTS);
        assert!(
            elapsed <= Duration::from_millis(300),
            "first page took {elapsed:?}"
        );
    }
}
