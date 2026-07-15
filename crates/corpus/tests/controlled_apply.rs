use corpus::tools::subagent::{
    ApplyAuthorization, ApplyAuthorizer, ApplyError, ApplySpec, ControlledApply,
};
use fabric::{ApprovalId, ApprovalStatus};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
struct MockAuthorizer {
    value: Arc<Mutex<Option<ApplyAuthorization>>>,
}

impl ApplyAuthorizer for MockAuthorizer {
    fn authorization(&self, _: ApprovalId) -> Result<Option<ApplyAuthorization>, String> {
        Ok(self.value.lock().unwrap().clone())
    }
}

struct Fixture {
    _temp: TempDir,
    repo: PathBuf,
    patch: PathBuf,
    report: PathBuf,
    spec: ApplySpec,
    authorization: Arc<Mutex<Option<ApplyAuthorization>>>,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-q"]);
        git(&repo, &["config", "user.email", "test@example.com"]);
        git(&repo, &["config", "user.name", "Test"]);
        std::fs::write(repo.join("allowed.txt"), "before\n").unwrap();
        std::fs::write(repo.join("unrelated.txt"), "stable\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-qm", "base"]);
        let head = output(&repo, &["rev-parse", "HEAD"]).trim().to_owned();

        std::fs::write(repo.join("allowed.txt"), "after\n").unwrap();
        let diff = Command::new("git")
            .args(["diff", "--binary", "--", "allowed.txt"])
            .current_dir(&repo)
            .output()
            .unwrap()
            .stdout;
        git(&repo, &["checkout", "--", "allowed.txt"]);
        let patch = temp.path().join("change.patch");
        let report = temp.path().join("verification.json");
        std::fs::write(&patch, &diff).unwrap();
        std::fs::write(&report, br#"{"checks":"passed"}"#).unwrap();
        let diff_sha256 = hash(&diff);
        let verification_sha256 = hash(&std::fs::read(&report).unwrap());
        let approval_id = ApprovalId::new();
        let subject_hash = "approved-subject".to_string();
        let allowed_paths = vec![PathBuf::from("allowed.txt")];
        let spec = ApplySpec {
            repository_root: repo.clone(),
            expected_head: head.clone(),
            diff_artifact: patch.clone(),
            diff_sha256: diff_sha256.clone(),
            verification_artifact: report.clone(),
            verification_sha256: verification_sha256.clone(),
            allowed_paths: allowed_paths.clone(),
            approval_id,
            subject_hash: subject_hash.clone(),
            timeout: Duration::from_secs(5),
            dry_run: false,
        };
        let authorization = Arc::new(Mutex::new(Some(ApplyAuthorization {
            approval_id,
            status: ApprovalStatus::Approved,
            subject_hash,
            expected_head: head,
            diff_sha256,
            verification_sha256,
            allowed_paths,
        })));
        Self {
            _temp: temp,
            repo,
            patch,
            report,
            spec,
            authorization,
        }
    }

    fn applier(&self) -> ControlledApply {
        ControlledApply::new(Arc::new(MockAuthorizer {
            value: self.authorization.clone(),
        }))
        .unwrap()
    }
}

#[tokio::test]
async fn applies_approved_patch_to_index_and_worktree() {
    let f = Fixture::new();
    let result = f
        .applier()
        .apply(f.spec.clone(), CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(f.repo.join("allowed.txt")).unwrap(),
        "after\n"
    );
    assert_eq!(result.changed_paths, vec![PathBuf::from("allowed.txt")]);
    assert!(output(&f.repo, &["diff", "--cached", "--name-only"]).contains("allowed.txt"));
}

#[tokio::test]
async fn dry_run_checks_without_mutating() {
    let f = Fixture::new();
    let mut spec = f.spec.clone();
    spec.dry_run = true;
    let result = f
        .applier()
        .apply(spec, CancellationToken::new())
        .await
        .unwrap();
    assert!(result.dry_run);
    assert_eq!(
        std::fs::read_to_string(f.repo.join("allowed.txt")).unwrap(),
        "before\n"
    );
    assert!(output(&f.repo, &["status", "--porcelain"]).is_empty());
}

#[tokio::test]
async fn rejected_expired_and_replayed_approvals_fail_closed() {
    for status in [
        ApprovalStatus::Rejected,
        ApprovalStatus::Expired,
        ApprovalStatus::Consumed,
    ] {
        let f = Fixture::new();
        f.authorization.lock().unwrap().as_mut().unwrap().status = status;
        assert!(matches!(
            f.applier()
                .apply(f.spec.clone(), CancellationToken::new())
                .await,
            Err(ApplyError::Unauthorized(_))
        ));
    }
}

#[tokio::test]
async fn stale_head_is_rejected() {
    let f = Fixture::new();
    std::fs::write(f.repo.join("new.txt"), "new\n").unwrap();
    git(&f.repo, &["add", "new.txt"]);
    git(&f.repo, &["commit", "-qm", "advance"]);
    assert!(matches!(
        f.applier()
            .apply(f.spec.clone(), CancellationToken::new())
            .await,
        Err(ApplyError::StaleHead { .. })
    ));
}

#[tokio::test]
async fn tampered_diff_and_report_are_rejected() {
    let f = Fixture::new();
    std::fs::write(&f.patch, "tampered").unwrap();
    assert!(matches!(
        f.applier()
            .apply(f.spec.clone(), CancellationToken::new())
            .await,
        Err(ApplyError::Artifact(_))
    ));

    let f = Fixture::new();
    std::fs::write(&f.report, "tampered").unwrap();
    assert!(matches!(
        f.applier()
            .apply(f.spec.clone(), CancellationToken::new())
            .await,
        Err(ApplyError::Artifact(_))
    ));
}

#[tokio::test]
async fn path_outside_approved_scope_is_rejected() {
    let f = Fixture::new();
    f.authorization
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .allowed_paths = vec![PathBuf::from("other")];
    assert!(matches!(
        f.applier()
            .apply(f.spec.clone(), CancellationToken::new())
            .await,
        Err(ApplyError::Unauthorized(_))
    ));
}

#[tokio::test]
async fn patch_path_escape_is_rejected() {
    let f = Fixture::new();
    let malicious = b"diff --git a/../escape.txt b/../escape.txt\nnew file mode 100644\nindex 0000000..3e75765\n--- /dev/null\n+++ b/../escape.txt\n@@ -0,0 +1 @@\n+escape\n";
    std::fs::write(&f.patch, malicious).unwrap();
    let digest = hash(malicious);
    let mut spec = f.spec.clone();
    spec.diff_sha256 = digest.clone();
    f.authorization
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .diff_sha256 = digest;
    assert!(matches!(
        f.applier().apply(spec, CancellationToken::new()).await,
        Err(ApplyError::Scope(_))
    ));
    assert!(!f._temp.path().join("escape.txt").exists());
}

#[tokio::test]
async fn symlink_patch_is_rejected() {
    let f = Fixture::new();
    let malicious = b"diff --git a/allowed.txt b/allowed.txt\nnew file mode 120000\nindex 0000000..1de5659\n--- /dev/null\n+++ b/allowed.txt\n@@ -0,0 +1 @@\n+../outside\n";
    std::fs::write(&f.patch, malicious).unwrap();
    let digest = hash(malicious);
    let mut spec = f.spec.clone();
    spec.diff_sha256 = digest.clone();
    f.authorization
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .diff_sha256 = digest;
    assert!(matches!(
        f.applier().apply(spec, CancellationToken::new()).await,
        Err(ApplyError::Scope(_))
    ));
}

#[tokio::test]
async fn conflict_leaves_worktree_and_index_unchanged() {
    let f = Fixture::new();
    std::fs::write(f.repo.join("allowed.txt"), "local change\n").unwrap();
    let before = output(&f.repo, &["status", "--porcelain=v2"]);
    assert!(matches!(
        f.applier()
            .apply(f.spec.clone(), CancellationToken::new())
            .await,
        Err(ApplyError::CheckFailed(_))
    ));
    assert_eq!(output(&f.repo, &["status", "--porcelain=v2"]), before);
    assert_eq!(
        std::fs::read_to_string(f.repo.join("allowed.txt")).unwrap(),
        "local change\n"
    );
}

#[tokio::test]
async fn cancellation_happens_before_mutation() {
    let f = Fixture::new();
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert!(matches!(
        f.applier().apply(f.spec.clone(), cancel).await,
        Err(ApplyError::Cancelled)
    ));
    assert_eq!(
        std::fs::read_to_string(f.repo.join("allowed.txt")).unwrap(),
        "before\n"
    );
}

#[tokio::test]
async fn dirty_unrelated_files_are_preserved() {
    let f = Fixture::new();
    std::fs::write(f.repo.join("unrelated.txt"), "user work\n").unwrap();
    f.applier()
        .apply(f.spec.clone(), CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(f.repo.join("unrelated.txt")).unwrap(),
        "user work\n"
    );
}

fn git(repo: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(repo)
        .status()
        .unwrap();
    assert!(status.success(), "git {:?} failed", args);
}

fn output(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {:?} failed", args);
    String::from_utf8(output.stdout).unwrap()
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
