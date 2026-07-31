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
    ".aletheon-validation.toml",
    "ARCHITECTURE.md",
    "docs/architecture.md",
    "docs/design/architecture-overview.md",
    "docs/STATUS.md",
    "docs/status.md",
    "docs/roadmap.md",
    "SECURITY.md",
    "CONTRIBUTING.md",
    ".github/workflows/ci.yml",
];
const ENTRY_ALTERNATIVES: &[&[&str]] = &[
    &["README.md", "README"],
    &[
        "ARCHITECTURE.md",
        "docs/architecture.md",
        "docs/design/architecture-overview.md",
        "docs/STATUS.md",
        "docs/status.md",
        "docs/roadmap.md",
    ],
];

pub struct RepoInspectTool;

#[async_trait]
impl Tool for RepoInspectTool {
    fn name(&self) -> &str {
        "repo_inspect"
    }

    fn description(&self) -> &str {
        "Required first inspection for an unfamiliar repository or workspace overview. Call it alone and wait for the result before further discovery. Batch-reads bounded known entry files and returns a versioned, content-backed RepositoryContext with instructions/manifests, exact follow-up paths, missing candidates, VCS state, and explicit evidence references. Use exact_follow_up_paths with file_read instead of wildcard glob discovery. Each missing_candidates item means only that exact candidate path was unavailable; it never proves that an alternative file, file category, capability, or parent directory is absent. entry_files is authoritative presence evidence."
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
            Ok(context) => {
                super::overview_guard::mark(ctx);
                ToolResult {
                    content: serde_json::to_string_pretty(&context).unwrap_or_else(|error| {
                        format!("repository context serialization failed: {error}")
                    }),
                    is_error: false,
                    metadata: ToolResultMeta {
                        execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                        truncated: false,
                        patch_delta: None,
                    },
                }
            }
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
    let found_paths = found
        .iter()
        .map(|(path, _)| path.as_str())
        .collect::<std::collections::HashSet<_>>();
    missing_candidates.retain(|missing| {
        let candidate = missing
            .split_once(" (")
            .map_or(missing.as_str(), |pair| pair.0);
        !ENTRY_ALTERNATIVES.iter().any(|group| {
            group.contains(&candidate) && group.iter().any(|path| found_paths.contains(path))
        })
    });

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
    let deployment_policy = match found
        .iter()
        .find(|(path, _)| path == ".aletheon-validation.toml")
    {
        Some((_, file)) => Some(parse_deployment_policy(file)?),
        None => None,
    };
    let vcs_state = vcs_snapshot(&root, &store);
    let exact_follow_up_paths = workspace_follow_up_paths(&root);
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
        evidence_constraints: vec![
            "Authorship metadata and commit history do not establish maintainer, contributor, or staffing count."
                .into(),
            "A version identifier alone does not establish production maturity or API stability."
                .into(),
            "Unavailable candidate paths are exact-path evidence only and cannot establish category or capability absence."
                .into(),
        ],
        exact_follow_up_paths,
        missing_candidates,
        vcs_state,
        validation_commands,
        protected_paths,
        // Deployment acceptance comes only from typed repository metadata.
        // Natural-language instruction phrases are never promoted implicitly.
        deployment_policy,
    };
    context.version = digest_json(&context)?;
    Ok(context)
}

fn workspace_follow_up_paths(root: &Path) -> Vec<String> {
    const MAX_PATHS: usize = 64;
    let Ok(content) = std::fs::read_to_string(root.join("Cargo.toml")) else {
        return Vec::new();
    };
    let Ok(document) = content.parse::<toml::Value>() else {
        return Vec::new();
    };
    let Some(members) = document
        .get("workspace")
        .and_then(|workspace| workspace.get("members"))
        .and_then(toml::Value::as_array)
    else {
        return Vec::new();
    };
    let mut paths = std::collections::BTreeSet::new();
    for member in members.iter().filter_map(toml::Value::as_str) {
        let manifest_pattern = root.join(member).join("Cargo.toml");
        let pattern = manifest_pattern.to_string_lossy();
        let candidates = match glob::glob(&pattern) {
            Ok(matches) => matches.filter_map(Result::ok).collect::<Vec<_>>(),
            Err(_) => Vec::new(),
        };
        for manifest in candidates {
            let Ok(canonical) = std::fs::canonicalize(&manifest) else {
                continue;
            };
            if !canonical.starts_with(root) {
                continue;
            }
            let Some(package_dir) = canonical.parent() else {
                continue;
            };
            for candidate in [
                canonical.clone(),
                package_dir.join("src/lib.rs"),
                package_dir.join("src/main.rs"),
            ] {
                if candidate.is_file() {
                    if let Ok(relative) = candidate.strip_prefix(root) {
                        paths.insert(relative.to_string_lossy().replace('\\', "/"));
                    }
                }
            }
        }
    }
    paths.into_iter().take(MAX_PATHS).collect()
}

#[derive(serde::Deserialize)]
struct ValidationPolicyFile {
    schema_version: u32,
    installed_runtime: Option<InstalledRuntimePolicy>,
}

#[derive(serde::Deserialize)]
struct InstalledRuntimePolicy {
    required: bool,
    command: String,
    #[serde(default)]
    affected_path_prefixes: Vec<String>,
}

fn parse_deployment_policy(
    file: &RepositoryFileEvidence,
) -> anyhow::Result<fabric::repository::DeploymentPolicy> {
    if file.preview_truncated {
        anyhow::bail!("validation policy exceeds bounded typed-metadata preview");
    }
    let policy: ValidationPolicyFile = toml::from_str(&file.preview)?;
    if policy.schema_version != 1 {
        anyhow::bail!("unsupported validation policy schema version");
    }
    let installed = policy
        .installed_runtime
        .ok_or_else(|| anyhow::anyhow!("installed_runtime policy is missing"))?;
    if installed.required && installed.command.trim().is_empty() {
        anyhow::bail!("installed-runtime command cannot be empty when required");
    }
    if installed
        .affected_path_prefixes
        .iter()
        .any(|prefix| prefix.is_empty() || Path::new(prefix).is_absolute() || prefix.contains(".."))
    {
        anyhow::bail!("installed-runtime path prefixes must be safe relative prefixes");
    }
    Ok(fabric::repository::DeploymentPolicy {
        source_path: file.path.clone(),
        requires_installed_runtime: installed.required,
        command: Some(installed.command),
        affected_path_prefixes: installed.affected_path_prefixes,
    })
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
            let kind = if command.contains("fmt") || command.contains("format") {
                "format"
            } else if command.contains(" test") || command.starts_with("test ") {
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
        assert!(result.exact_follow_up_paths.is_empty());
        assert!(result
            .evidence_constraints
            .iter()
            .any(|constraint| constraint.contains("staffing count")));
    }

    #[test]
    fn inspection_returns_exact_workspace_follow_up_paths() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join("crates/demo/src")).unwrap();
        std::fs::write(
            temp.path().join("Cargo.toml"),
            "[workspace]\nmembers=['crates/*']\n",
        )
        .unwrap();
        std::fs::write(
            temp.path().join("crates/demo/Cargo.toml"),
            "[package]\nname='demo'\nversion='0.1.0'\n",
        )
        .unwrap();
        std::fs::write(
            temp.path().join("crates/demo/src/lib.rs"),
            "pub fn demo() {}\n",
        )
        .unwrap();
        let context = ToolContext {
            agent: None,
            approval_authority: None,
            working_dir: temp.path().to_path_buf(),
            session_id: "repo-follow-up-test".into(),
            clock: Arc::new(kernel::chronos::TestClock::default()),
            turn_event_sender: None,
        };

        let result = inspect(json!({}), &context).unwrap();

        assert_eq!(
            result.exact_follow_up_paths,
            ["crates/demo/Cargo.toml", "crates/demo/src/lib.rs"]
        );
    }

    #[test]
    fn inspection_does_not_report_missing_alternative_when_category_is_present() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("README.md"), "# Present\n").unwrap();
        std::fs::create_dir_all(temp.path().join("docs/design")).unwrap();
        std::fs::write(
            temp.path().join("docs/design/architecture-overview.md"),
            "# Architecture\n",
        )
        .unwrap();
        let context = ToolContext {
            agent: None,
            approval_authority: None,
            working_dir: temp.path().to_path_buf(),
            session_id: "repo-alternative-test".into(),
            clock: Arc::new(kernel::chronos::TestClock::default()),
            turn_event_sender: None,
        };

        let result = inspect(json!({}), &context).unwrap();

        assert!(!result
            .missing_candidates
            .iter()
            .any(|path| path == "README"));
        assert!(!result.missing_candidates.iter().any(|path| {
            matches!(
                path.as_str(),
                "ARCHITECTURE.md"
                    | "docs/architecture.md"
                    | "docs/STATUS.md"
                    | "docs/status.md"
                    | "docs/roadmap.md"
            )
        }));
    }

    #[test]
    fn inspection_loads_typed_installed_runtime_policy() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join(".aletheon-validation.toml"),
            "schema_version=1\n[installed_runtime]\nrequired=true\ncommand='sudo deploy'\naffected_path_prefixes=['crates/runtime']\n",
        )
        .unwrap();
        let context = ToolContext {
            agent: None,
            approval_authority: None,
            working_dir: temp.path().to_path_buf(),
            session_id: "repo-policy-test".into(),
            clock: Arc::new(kernel::chronos::TestClock::default()),
            turn_event_sender: None,
        };
        let result = inspect(json!({}), &context).unwrap();
        let policy = result.deployment_policy.unwrap();
        assert!(policy.requires_installed_runtime);
        assert_eq!(policy.command.as_deref(), Some("sudo deploy"));
        assert_eq!(policy.affected_path_prefixes, vec!["crates/runtime"]);
    }

    #[test]
    fn malformed_typed_validation_policy_fails_closed() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join(".aletheon-validation.toml"),
            "schema_version=1\n[installed_runtime]\nrequired=true\ncommand=''\n",
        )
        .unwrap();
        let context = ToolContext {
            agent: None,
            approval_authority: None,
            working_dir: temp.path().to_path_buf(),
            session_id: "repo-policy-test".into(),
            clock: Arc::new(kernel::chronos::TestClock::default()),
            turn_event_sender: None,
        };
        assert!(inspect(json!({}), &context).is_err());
    }
}
