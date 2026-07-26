//! Bounded, content-backed repository overview.

use async_trait::async_trait;
use fabric::repository::{
    InstructionSource, ManifestRef, RepositoryContext, RepositoryFileEvidence, ValidationSpec,
    VcsSnapshot,
};
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

use crate::tools::artifact::ArtifactStore;

use super::{ConcurrencyClass, PermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta};

const PREVIEW_BYTES: usize = 16 * 1024;
const MAX_EVIDENCE_BYTES: usize = 512 * 1024;
const ENTRY_CANDIDATES: &[&str] = &[
    "AGENTS.md",
    "CLAUDE.md",
    "README.md",
    "README",
    "Cargo.toml",
    "package.json",
    "pyproject.toml",
    "go.mod",
    "Makefile",
];

pub struct RepoInspectTool;

#[async_trait]
impl Tool for RepoInspectTool {
    fn name(&self) -> &str {
        "repo_inspect"
    }

    fn description(&self) -> &str {
        "Inspect a repository by batch-reading bounded known entry files. Returns a versioned, content-backed RepositoryContext with instruction/manifests, missing candidates, VCS state, and explicit evidence references."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "root": {"type": "string", "description": "Repository root; defaults to the governed working directory"}
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L0
    }

    fn concurrency_class(&self) -> ConcurrencyClass {
        ConcurrencyClass::ReadOnly
    }

    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(Self)
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let start = ctx.clock.mono_now();
        match inspect(input, ctx) {
            Ok(context) => ToolResult {
                content: serde_json::to_string_pretty(&context).unwrap_or_else(|error| {
                    format!("repository context serialization failed: {error}")
                }),
                is_error: false,
                metadata: ToolResultMeta {
                    execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                    truncated: false,
                    patch_delta: None,
                },
            },
            Err(error) => ToolResult {
                content: format!("repository inspection failed: {error}"),
                is_error: true,
                metadata: ToolResultMeta {
                    execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                    truncated: false,
                    patch_delta: None,
                },
            },
        }
    }
}

fn inspect(input: serde_json::Value, ctx: &ToolContext) -> anyhow::Result<RepositoryContext> {
    let workspace = ctx
        .effective_workspace_policy()
        .map_err(anyhow::Error::msg)?;
    let requested = input
        .get("root")
        .and_then(|value| value.as_str())
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace.cwd().to_path_buf());
    let requested = if requested.is_absolute() {
        requested
    } else {
        workspace.cwd().join(requested)
    };
    let root = std::fs::canonicalize(&requested)?;
    if !root.starts_with(workspace.cwd())
        && !workspace
            .writable_roots()
            .iter()
            .any(|authority| root.starts_with(authority))
    {
        anyhow::bail!("root is outside the governed workspace");
    }

    let store = ArtifactStore::new(
        super::output::OutputConfig::default()
            .overflow_dir
            .join("artifacts"),
    );
    let mut found = Vec::new();
    let mut missing_candidates = Vec::new();
    for candidate in ENTRY_CANDIDATES {
        let path = root.join(candidate);
        match std::fs::read(&path) {
            Ok(bytes) if bytes.len() <= MAX_EVIDENCE_BYTES => {
                found.push((candidate.to_string(), evidence(candidate, &bytes, &store)?));
            }
            Ok(_) => missing_candidates.push(format!("{candidate} (exceeds bounded read limit)")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                missing_candidates.push(candidate.to_string());
            }
            Err(error) => missing_candidates.push(format!("{candidate} ({error})")),
        }
    }

    let instructions = found
        .iter()
        .filter(|(path, _)| matches!(path.as_str(), "AGENTS.md" | "CLAUDE.md"))
        .enumerate()
        .map(|(index, (_, file))| InstructionSource {
            file: file.clone(),
            precedence: index as u32,
        })
        .collect::<Vec<_>>();
    let manifests = found
        .iter()
        .filter_map(|(path, file)| {
            manifest_kind(path).map(|kind| ManifestRef {
                file: file.clone(),
                kind: kind.into(),
            })
        })
        .collect::<Vec<_>>();
    let validation_commands = instructions
        .iter()
        .flat_map(extract_validation_specs)
        .collect::<Vec<_>>();
    let vcs_state = vcs_snapshot(&root, &store);
    let entry_files = found.into_iter().map(|(_, file)| file).collect();
    let protected_paths = workspace
        .protected_paths()
        .credential_paths()
        .iter()
        .map(|path| path.display().to_string())
        .collect();
    let mut context = RepositoryContext {
        root: root.display().to_string(),
        version: String::new(),
        instructions,
        manifests,
        entry_files,
        missing_candidates,
        vcs_state,
        validation_commands,
        protected_paths,
        // Deployment acceptance must come from typed repository metadata. Do
        // not infer it from natural-language instruction phrases.
        deployment_policy: None,
    };
    context.version = digest_json(&context)?;
    Ok(context)
}

fn evidence(
    path: &str,
    bytes: &[u8],
    store: &ArtifactStore,
) -> anyhow::Result<RepositoryFileEvidence> {
    let artifact = store.store(bytes, "text/plain; charset=utf-8")?;
    let preview_end = bytes.len().min(PREVIEW_BYTES);
    Ok(RepositoryFileEvidence {
        path: path.into(),
        sha256: artifact.sha256.clone(),
        size_bytes: bytes.len() as u64,
        artifact_ref: artifact.uri(),
        preview: String::from_utf8_lossy(&bytes[..preview_end]).into_owned(),
        preview_truncated: preview_end < bytes.len(),
    })
}

fn manifest_kind(path: &str) -> Option<&'static str> {
    match path {
        "Cargo.toml" => Some("cargo"),
        "package.json" => Some("npm"),
        "pyproject.toml" => Some("python"),
        "go.mod" => Some("go"),
        "Makefile" => Some("make"),
        _ => None,
    }
}

fn extract_validation_specs(source: &InstructionSource) -> Vec<ValidationSpec> {
    source
        .file
        .preview
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let command = line
                .split('`')
                .enumerate()
                .find_map(|(part, value)| (part % 2 == 1).then_some(value.trim()))?;
            let kind = if command.contains(" test") || command.starts_with("test ") {
                "test"
            } else if command.contains("check") {
                "check"
            } else if command.contains("lint") || command.contains("clippy") {
                "lint"
            } else if command.contains("deploy") {
                "deploy"
            } else if command.contains("build") {
                "build"
            } else {
                return None;
            };
            Some(ValidationSpec {
                kind: kind.into(),
                command: command.into(),
                source_path: source.file.path.clone(),
                source_line: (index + 1) as u32,
            })
        })
        .collect()
}

fn vcs_snapshot(root: &Path, store: &ArtifactStore) -> VcsSnapshot {
    let output = std::process::Command::new("git")
        .args(["status", "--porcelain=v1", "--branch"])
        .current_dir(root)
        .output();
    let Ok(output) = output else {
        return VcsSnapshot::default();
    };
    if !output.status.success() {
        return VcsSnapshot::default();
    }
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    let first = text.lines().next().unwrap_or_default();
    let branch = first
        .strip_prefix("## ")
        .map(|value| value.split(['.', ' ']).next().unwrap_or(value).to_string());
    let artifact_ref = store
        .store(text.as_bytes(), "text/plain; charset=utf-8")
        .ok()
        .map(|artifact| artifact.uri());
    VcsSnapshot {
        kind: Some("git".into()),
        head: read_git_head(root),
        branch,
        dirty: Some(text.lines().skip(1).next().is_some()),
        status_artifact_ref: artifact_ref,
    }
}

fn read_git_head(root: &Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(root)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn digest_json<T: Serialize>(value: &T) -> anyhow::Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn inspection_batches_known_content_into_versioned_evidence() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("AGENTS.md"),
            "Validate with `bash scripts/check.sh test`.\n",
        )
        .unwrap();
        std::fs::write(temp.path().join("Cargo.toml"), "[workspace]\n").unwrap();
        let context = ToolContext {
            agent: None,
            approval_authority: None,
            working_dir: temp.path().to_path_buf(),
            session_id: "repo-inspect-test".into(),
            clock: Arc::new(kernel::chronos::TestClock::default()),
            turn_event_sender: None,
        };

        let result = inspect(json!({}), &context).unwrap();

        assert_eq!(result.version.len(), 64);
        assert_eq!(result.instructions.len(), 1);
        assert_eq!(result.manifests.len(), 1);
        assert_eq!(result.validation_commands.len(), 1);
        assert_eq!(result.validation_commands[0].kind, "test");
        assert!(result
            .entry_files
            .iter()
            .all(|file| file.artifact_ref.starts_with("artifact://sha256/")));
        assert!(result.missing_candidates.contains(&"README.md".into()));
    }
}
