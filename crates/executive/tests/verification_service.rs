use executive::service::verification::{
    ArchitecturePolicy, CapabilityAuditSummary, ForbiddenDependencyEdge, VerificationCheckKind,
    VerificationContext, VerificationSelection, VerificationService, VerificationServiceConfig,
};
use fabric::{
    AttemptId, ChangedFile, ChangedFileKind, CodingJobId, GoalId, VerificationCheck,
    VerificationReport,
};
use kernel::chronos::SystemClock;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

struct Fixture {
    _temp: TempDir,
    repo: PathBuf,
    control: PathBuf,
    cargo: PathBuf,
    git: PathBuf,
    base_commit: String,
}

impl Fixture {
    fn new() -> Self {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("repo");
        let control = temp.path().join("control");
        std::fs::create_dir_all(repo.join("src")).unwrap();
        std::fs::create_dir(&control).unwrap();
        std::fs::write(
            repo.join("Cargo.toml"),
            "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        std::fs::write(repo.join("src/lib.rs"), "pub fn value() -> u8 { 1 }\n").unwrap();
        git(&repo, &["init", "-q"]);
        git(&repo, &["config", "user.email", "verify@test.invalid"]);
        git(&repo, &["config", "user.name", "Verify Test"]);
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-qm", "base"]);
        let base_commit = git_output(&repo, &["rev-parse", "HEAD"]);
        std::fs::write(repo.join("src/lib.rs"), "pub fn value() -> u8 { 2 }\n").unwrap();

        let cargo = temp.path().join("trusted-cargo");
        std::fs::write(
            &cargo,
            r#"#!/bin/sh
set -eu
kind="$1"
printf '%s\n' "$*" >> "$CONTROL/calls"
printf started > "$CONTROL/started-$kind"
mode=pass
if [ -f "$CONTROL/mode-$kind" ]; then mode=$(cat "$CONTROL/mode-$kind"); fi
case "$mode" in
  pass) printf '%s passed\n' "$kind" ;;
  fail) printf '%s failed\n' "$kind" >&2; exit 9 ;;
  timeout) sleep 30 ;;
  output) head -c 5000 /dev/zero | tr '\000' x ;;
  *) exit 19 ;;
esac
"#,
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&cargo, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let git = which::which("git").unwrap().canonicalize().unwrap();
        Self {
            _temp: temp,
            repo: repo.canonicalize().unwrap(),
            control,
            cargo: cargo.canonicalize().unwrap(),
            git,
            base_commit,
        }
    }

    fn config(&self, timeout: Duration, output_cap_bytes: usize) -> VerificationServiceConfig {
        VerificationServiceConfig {
            cargo_program: self.cargo.clone(),
            git_program: self.git.clone(),
            compile_args: vec!["check".into(), "--workspace".into()],
            relevant_test_args: vec![vec!["test".into(), "-p".into(), "fixture".into()]],
            command_timeout: timeout,
            output_cap_bytes,
            environment: BTreeMap::from([
                ("PATH".into(), "/usr/bin:/bin".into()),
                (
                    "CONTROL".into(),
                    self.control.to_string_lossy().into_owned(),
                ),
            ]),
            architecture: ArchitecturePolicy::default(),
        }
    }

    fn context(&self, changed_files: Vec<ChangedFile>) -> VerificationContext {
        VerificationContext {
            job_id: CodingJobId::new(),
            goal_id: GoalId(9),
            attempt_id: AttemptId::new(),
            worktree: self.repo.clone(),
            base_commit: self.base_commit.clone(),
            changed_files,
            allowed_paths: vec![
                PathBuf::from("src"),
                PathBuf::from("Cargo.toml"),
                PathBuf::from("docs"),
            ],
            forbidden_paths: vec![PathBuf::from(".git"), PathBuf::from("secrets")],
            capability_audit: CapabilityAuditSummary {
                audit_present: true,
                observed_capabilities: vec!["file.write".into()],
                allowed_capabilities: vec!["file.write".into()],
                unavailable_capabilities: vec![],
            },
            selection: VerificationSelection::default(),
        }
    }

    fn default_context(&self) -> VerificationContext {
        self.context(vec![changed("src/lib.rs", ChangedFileKind::Modified)])
    }

    fn set_mode(&self, command: &str, mode: &str) {
        std::fs::write(self.control.join(format!("mode-{command}")), mode).unwrap();
    }

    fn started(&self, command: &str) -> bool {
        self.control.join(format!("started-{command}")).exists()
    }
}

fn git(repository: &Path, args: &[&str]) {
    assert!(Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(args)
        .status()
        .unwrap()
        .success());
}

fn git_output(repository: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap().trim().into()
}

fn changed(path: &str, kind: ChangedFileKind) -> ChangedFile {
    ChangedFile {
        path: PathBuf::from(path),
        kind,
        before_bytes: 1,
        after_bytes: 1,
        content_sha256: "00".repeat(32),
    }
}

fn check(report: &VerificationReport, kind: VerificationCheckKind) -> &VerificationCheck {
    report
        .checks
        .iter()
        .find(|check| check.name == kind.as_str())
        .unwrap()
}

fn command_name(kind: VerificationCheckKind) -> &'static str {
    match kind {
        VerificationCheckKind::Format => "fmt",
        VerificationCheckKind::Compile => "check",
        VerificationCheckKind::RelevantTests => "test",
        VerificationCheckKind::Clippy => "clippy",
        _ => panic!("not a cargo command check"),
    }
}

const COMMAND_CHECKS: [VerificationCheckKind; 4] = [
    VerificationCheckKind::Format,
    VerificationCheckKind::Compile,
    VerificationCheckKind::RelevantTests,
    VerificationCheckKind::Clippy,
];

#[tokio::test]
async fn trusted_commands_use_exact_argv_and_full_report_passes() {
    let fixture = Fixture::new();
    let service = VerificationService::with_clock(
        fixture.config(Duration::from_secs(2), 4096),
        Arc::new(SystemClock::new()),
    )
    .unwrap();
    let report = service
        .verify(&fixture.default_context(), CancellationToken::new())
        .await
        .unwrap();
    assert!(report.passed);
    let calls = std::fs::read_to_string(fixture.control.join("calls")).unwrap();
    assert!(calls.lines().any(|line| line == "fmt --all -- --check"));
    assert!(calls.lines().any(|line| line == "check --workspace"));
    assert!(calls.lines().any(|line| line == "test -p fixture"));
    assert!(calls
        .lines()
        .any(|line| line == "clippy --workspace --all-targets -- -D warnings"));
}

#[tokio::test]
async fn every_cargo_check_reports_failure_timeout_and_output_truncation() {
    for kind in COMMAND_CHECKS {
        for mode in ["fail", "timeout", "output"] {
            let fixture = Fixture::new();
            fixture.set_mode(command_name(kind), mode);
            let service = VerificationService::with_clock(
                // Process startup plus the output-producing shell pipeline can
                // exceed 40 ms on a busy workspace runner. Keep this far below
                // the fixture's 30-second hang while avoiding false timeouts.
                fixture.config(Duration::from_millis(250), 64),
                Arc::new(SystemClock::new()),
            )
            .unwrap();
            let report = service
                .verify(&fixture.default_context(), CancellationToken::new())
                .await
                .unwrap();
            let result = check(&report, kind);
            match mode {
                "fail" => {
                    assert!(!result.passed, "{kind:?}");
                    assert!(!result.timed_out);
                }
                "timeout" => {
                    assert!(result.timed_out, "{kind:?}");
                    assert!(!result.passed);
                }
                "output" => {
                    assert!(result.passed, "{kind:?}");
                    assert!(
                        result
                            .evidence
                            .iter()
                            .any(|item| item == "stdout truncated"),
                        "{kind:?}: {:?}",
                        result.evidence
                    );
                }
                _ => unreachable!(),
            }
        }
    }
}

#[tokio::test]
async fn every_cargo_check_is_cancellable() {
    for kind in COMMAND_CHECKS {
        let fixture = Fixture::new();
        let command = command_name(kind);
        fixture.set_mode(command, "timeout");
        let service = VerificationService::with_clock(
            fixture.config(Duration::from_secs(30), 128),
            Arc::new(SystemClock::new()),
        )
        .unwrap();
        let context = fixture.default_context();
        let cancel = CancellationToken::new();
        let task = tokio::spawn({
            let cancel = cancel.clone();
            async move { service.verify(&context, cancel).await }
        });
        for _ in 0..200 {
            if fixture.started(command) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(fixture.started(command), "{kind:?} did not start");
        cancel.cancel();
        let report = task.await.unwrap().unwrap();
        assert!(check(&report, kind).cancelled, "{kind:?}");
    }
}

#[tokio::test]
async fn diff_scope_uses_fresh_status_and_enforces_path_policy() {
    let fixture = Fixture::new();
    let service = VerificationService::new(fixture.config(Duration::from_secs(2), 4096)).unwrap();
    let report = service
        .verify(&fixture.default_context(), CancellationToken::new())
        .await
        .unwrap();
    assert!(check(&report, VerificationCheckKind::DiffScope).passed);

    std::fs::write(fixture.repo.join("src/unreported.rs"), "// new\n").unwrap();
    let report = service
        .verify(&fixture.default_context(), CancellationToken::new())
        .await
        .unwrap();
    assert!(!check(&report, VerificationCheckKind::DiffScope).passed);

    std::fs::create_dir_all(fixture.repo.join("secrets")).unwrap();
    std::fs::write(fixture.repo.join("secrets/token"), "secret").unwrap();
    let mut context = fixture.context(vec![
        changed("src/lib.rs", ChangedFileKind::Modified),
        changed("src/unreported.rs", ChangedFileKind::Added),
        changed("secrets/token", ChangedFileKind::Added),
    ]);
    context.allowed_paths.push(PathBuf::from("secrets"));
    let report = service
        .verify(&context, CancellationToken::new())
        .await
        .unwrap();
    let scope = check(&report, VerificationCheckKind::DiffScope);
    assert!(!scope.passed);
    assert!(scope.evidence.iter().any(|item| item.contains("forbidden")));
}

#[tokio::test]
async fn missing_or_disallowed_capability_audit_is_required_failure() {
    let fixture = Fixture::new();
    let service = VerificationService::new(fixture.config(Duration::from_secs(2), 4096)).unwrap();
    let mut context = fixture.default_context();
    context.capability_audit.audit_present = false;
    let report = service
        .verify(&context, CancellationToken::new())
        .await
        .unwrap();
    assert!(!check(&report, VerificationCheckKind::CapabilityPolicy).passed);
    assert!(!report.passed);

    context.capability_audit.audit_present = true;
    context.capability_audit.observed_capabilities = vec!["network.open".into()];
    let report = service
        .verify(&context, CancellationToken::new())
        .await
        .unwrap();
    assert!(!check(&report, VerificationCheckKind::CapabilityPolicy).passed);
}

#[tokio::test]
async fn architecture_rules_cover_paths_imports_and_dependency_direction_as_advisory() {
    let fixture = Fixture::new();
    std::fs::write(
        fixture.repo.join("src/lib.rs"),
        "use forbidden::layer;\npub fn value() -> u8 { 2 }\n",
    )
    .unwrap();
    std::fs::write(
        fixture.repo.join("Cargo.toml"),
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\n[dependencies]\nexecutive = \"1\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(fixture.repo.join("docs/arch")).unwrap();
    std::fs::write(fixture.repo.join("docs/arch/locked.md"), "changed\n").unwrap();
    let mut config = fixture.config(Duration::from_secs(2), 4096);
    config.architecture = ArchitecturePolicy {
        forbidden_path_prefixes: vec![PathBuf::from("docs/arch")],
        forbidden_import_prefixes: vec!["forbidden::".into()],
        forbidden_dependency_edges: vec![ForbiddenDependencyEdge {
            from: "fixture".into(),
            to: "executive".into(),
        }],
    };
    let service = VerificationService::new(config).unwrap();
    let context = fixture.context(vec![
        changed("src/lib.rs", ChangedFileKind::Modified),
        changed("Cargo.toml", ChangedFileKind::Modified),
        changed("docs/arch/locked.md", ChangedFileKind::Added),
    ]);
    let report = service
        .verify(&context, CancellationToken::new())
        .await
        .unwrap();
    let architecture = check(&report, VerificationCheckKind::ArchitectureReview);
    assert!(!architecture.passed);
    assert_eq!(architecture.evidence.len(), 3);
    assert!(report.passed, "advisory findings must not block passage");
    assert_eq!(report.risk_summary.len(), 1);
}

#[tokio::test]
async fn relevant_tests_require_explicit_trusted_argv() {
    let fixture = Fixture::new();
    let mut config = fixture.config(Duration::from_secs(2), 4096);
    config.relevant_test_args.clear();
    let service = VerificationService::new(config).unwrap();
    let report = service
        .verify(&fixture.default_context(), CancellationToken::new())
        .await
        .unwrap();
    assert!(!check(&report, VerificationCheckKind::RelevantTests).passed);
    assert!(!report.passed);
}
