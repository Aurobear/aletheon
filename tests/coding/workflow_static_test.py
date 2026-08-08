#!/usr/bin/env python3
"""Static contracts for coding-related GitHub Actions workflows."""

from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[2]


class OrdinaryCiWorkflowTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.workflow = (ROOT / ".github/workflows/ci.yml").read_text()

    def test_has_deterministic_coding_evidence_job(self) -> None:
        self.assertIn("name: Strict contracts", self.workflow)
        self.assertIn("python3 tests/coding/replay_test.py", self.workflow)
        self.assertIn("bash tests/coding/static_test.sh", self.workflow)

    def test_has_linux_platform_contract_job(self) -> None:
        self.assertIn("name: Verify Linux platform contracts", self.workflow)
        self.assertIn(
            "bash scripts/cargo-agent.sh +stable test -p platform --test contract_suite",
            self.workflow,
        )

    def test_remains_secret_free(self) -> None:
        self.assertNotIn("LEJU_API_KEY", self.workflow)


class RealEvaluationWorkflowTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.workflow = (ROOT / ".github/workflows/coding-e2e.yml").read_text()

    def test_is_manual_only(self) -> None:
        self.assertIn("  workflow_dispatch:", self.workflow)
        self.assertNotIn("  push:", self.workflow)
        self.assertNotIn("  pull_request:", self.workflow)
        self.assertNotIn("  schedule:", self.workflow)

    def test_pins_provider_model_and_secret_source(self) -> None:
        self.assertIn("ALETHEON_PROVIDER: lejurobot_deepseek", self.workflow)
        self.assertIn("ALETHEON_MODEL: deepseek/deepseek-v4-flash[1m]", self.workflow)
        self.assertIn("LEJU_API_KEY: ${{ secrets.LEJU_API_KEY }}", self.workflow)
        self.assertNotIn("echo $LEJU_API_KEY", self.workflow)
        self.assertNotIn("echo \"$LEJU_API_KEY\"", self.workflow)

    def test_runs_the_strict_suite_and_preserves_all_artifacts(self) -> None:
        self.assertIn("tests/coding/harness/suite.py", self.workflow)
        self.assertIn("--catalog tests/coding/tasks", self.workflow)
        self.assertIn('--receipts "$artifacts/receipts"', self.workflow)
        self.assertIn('--report "$artifacts/suite.json"', self.workflow)
        self.assertNotIn("for task in rust_bugfix", self.workflow)
        self.assertIn("actions/upload-artifact@v4", self.workflow)
        self.assertIn("if: always()", self.workflow)

    def test_acceptance_uses_only_installed_runtime_and_official_socket(self) -> None:
        self.assertIn("ALETHEON_BIN: /usr/bin/aletheon", self.workflow)
        self.assertIn("sudo bash scripts/aletheon.sh deploy", self.workflow)
        self.assertIn("ALETHEON_ACCEPTANCE_SOCKET", self.workflow)
        self.assertIn('--run-id "$run_id"', self.workflow)
        self.assertIn('--generation-id "$generation_id"', self.workflow)
        self.assertIn('--artifacts-root "$acceptance_root"', self.workflow)
        self.assertNotIn("target/debug/aletheon", self.workflow)
        self.assertNotIn('"$ALETHEON_BIN" core', self.workflow)

    def test_preserves_runner_toolchain_for_fixture_homes(self) -> None:
        self.assertIn('export RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"', self.workflow)
        self.assertIn('export CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"', self.workflow)

    def test_disables_unusable_runner_sandbox_only_for_manual_evaluation(self) -> None:
        self.assertIn("ALETHEON_CODING_SANDBOX: forbid", self.workflow)

    def test_preserves_typed_suite_exit_codes_without_inline_reclassification(self) -> None:
        self.assertIn("suite_exit=$?", self.workflow)
        self.assertIn('if (( suite_exit == 2 )); then', self.workflow)
        self.assertIn('if (( suite_exit == 1 )); then', self.workflow)
        self.assertNotIn('receipt.get("operation_id")', self.workflow)

    def test_never_uploads_the_credential_bearing_config_or_temporary_home(self) -> None:
        self.assertIn("path: ${{ runner.temp }}/coding-artifacts", self.workflow)
        self.assertNotIn("path: $HOME/.aletheon", self.workflow)
        self.assertNotIn("path: /etc/aletheon", self.workflow)


class ReleaseWorkflowTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.workflow = (ROOT / ".github/workflows/release.yml").read_text()

    # -- source identity / semver checks (validate job) -------------------

    def test_validate_has_semver_tag_validation(self) -> None:
        self.assertIn("Validate semver tag", self.workflow)
        self.assertIn('grep -qE "$SEMVER_RE"', self.workflow)

    def test_validate_checks_tag_commit_equals_origin_main(self) -> None:
        self.assertIn("Verify tag commit equals origin/main", self.workflow)
        self.assertIn("git rev-parse origin/main", self.workflow)
        self.assertIn('"$TAG_COMMIT" != "$MAIN_COMMIT"', self.workflow)

    def test_validate_has_explicit_ref_fetch(self) -> None:
        self.assertIn("Fetch origin/main and origin/dev refs explicitly", self.workflow)
        self.assertIn("refs/heads/main:refs/remotes/origin/main", self.workflow)
        self.assertIn("refs/heads/dev:refs/remotes/origin/dev", self.workflow)

    def test_validate_checks_origin_dev_is_ancestor_of_main(self) -> None:
        self.assertIn("Verify origin/dev is ancestor of main", self.workflow)
        self.assertIn("git merge-base --is-ancestor", self.workflow)
        self.assertIn('"$DEV_COMMIT"', self.workflow)

    def test_validate_includes_workspace_clippy(self) -> None:
        self.assertIn("Workspace clippy", self.workflow)
        self.assertIn("clippy --workspace --all-targets --all-features -- -D warnings", self.workflow)

    def test_validate_includes_workspace_tests(self) -> None:
        self.assertIn("Workspace tests", self.workflow)
        self.assertIn("test --workspace --all-features", self.workflow)

    def test_validate_includes_architecture_checks(self) -> None:
        self.assertIn("Architecture checks", self.workflow)
        self.assertIn(
            "bash scripts/libexec/aletheon/architecture-check.sh",
            self.workflow,
        )

    def test_validate_includes_migration_matrix_verification(self) -> None:
        self.assertIn("Migration matrix verification", self.workflow)
        self.assertIn(
            "bash scripts/libexec/aletheon/verify/migration-matrix.sh",
            self.workflow,
        )

    def test_validate_no_concurrent_workspace_builds(self) -> None:
        # All steps in validate are sequential; no parallel matrix or
        # background processes that would produce concurrent cargo
        # invocations within the same job.
        self.assertNotIn("strategy:", self.workflow[:self.workflow.index("build:")])

    # -- convergence-gate invariants ---------------------------------------

    def test_convergence_gate_downloads_r8_evidence_and_verifies(self) -> None:
        self.assertIn("Download R8 evidence for this exact commit", self.workflow)
        self.assertIn("r8-evidence-${GITHUB_SHA}", self.workflow)
        self.assertIn(
            "python3 tests/coding/harness/r8_evidence_verifier.py",
            self.workflow,
        )
        self.assertIn("--metadata", self.workflow)
        self.assertIn("r8-metadata.json", self.workflow)
        self.assertIn("--positive-receipt", self.workflow)
        self.assertIn("--negative-receipt", self.workflow)
        self.assertIn("--expected-commit \"${GITHUB_SHA}\"", self.workflow)
        self.assertIn("--expected-run-id \"${GITHUB_RUN_ID}\"", self.workflow)
        self.assertIn("--expected-rc-archive", self.workflow)
        self.assertIn("--expected-installed-digest", self.workflow)
        self.assertIn("--expected-device", self.workflow)
        # Must not use the old sha-only/device-only flags without metadata
        self.assertNotIn("--expected-device kuavo-mujoco-01", self.workflow)

    def test_convergence_gate_downloads_r8_evidence_deterministically(self) -> None:
        # Must use gh run list with r8-evidence-handoff.yml workflow name
        self.assertIn("gh run list", self.workflow)
        self.assertIn("r8-evidence-handoff.yml", self.workflow)
        self.assertIn("handoff_run_id", self.workflow)

    def test_convergence_gate_fails_closed_on_missing_r8_evidence(self) -> None:
        self.assertIn("no successful r8-evidence-handoff run found", self.workflow)
        self.assertIn("Run the r8-evidence-handoff workflow for this SHA before tagging.", self.workflow)

    def test_convergence_gate_downloads_x86_64_artifact(self) -> None:
        self.assertIn("actions/download-artifact@v4", self.workflow)
        self.assertIn(
            "pattern: aletheon-*-x86_64-unknown-linux-gnu.tar.gz",
            self.workflow,
        )

    def test_convergence_gate_download_merges_multiple(self) -> None:
        # actions/download-artifact v4 with pattern must use
        # merge-multiple: true so archives land flat instead of in
        # per-artifact directories.
        self.assertIn("merge-multiple: true", self.workflow)

    def test_convergence_gate_invokes_verifier_with_commit_and_archive(self) -> None:
        self.assertIn(
            "python3 tests/coding/harness/release_acceptance_gate.py",
            self.workflow,
        )
        self.assertIn("--commit \"${GITHUB_SHA}\"", self.workflow)
        self.assertIn("--x86-64-archive \"${archives[0]}\"", self.workflow)
        self.assertIn(
            '--run-dir "$run_dir"',
            self.workflow,
        )

    def test_convergence_gate_installs_and_accepts_exact_rc_without_rebuild(self) -> None:
        self.assertIn("target/release/aletheon", self.workflow)
        self.assertIn("sudo bash scripts/aletheon.sh deploy --no-build", self.workflow)
        self.assertIn("ALETHEON_BIN: /usr/bin/aletheon", self.workflow)
        self.assertIn("ALETHEON_ACCEPTANCE_SOCKET", self.workflow)
        self.assertIn("tests/coding/harness/suite.py", self.workflow)
        self.assertNotIn("artifacts/acceptance/${GITHUB_REF_NAME}", self.workflow)

    def test_convergence_gate_captures_acceptance_metadata(self) -> None:
        self.assertIn("Capture acceptance metadata", self.workflow)
        self.assertIn("release-acceptance-metadata.json", self.workflow)
        self.assertIn("acceptance_run_id", self.workflow)

    def test_convergence_gate_uploads_acceptance_metadata_artifact(self) -> None:
        self.assertIn("release-acceptance-metadata-${{ github.ref_name }}", self.workflow)

    def test_release_publishes_machine_generated_acceptance_archive(self) -> None:
        self.assertIn("release-acceptance-${{ github.ref_name }}", self.workflow)
        self.assertIn("acceptance-*.tar.gz", self.workflow)

    def test_convergence_gate_uploads_r8_evidence_bundle(self) -> None:
        self.assertIn("Upload R8 evidence bundle", self.workflow)
        self.assertIn("robot-r8-evidence-${{ github.ref_name }}", self.workflow)
        self.assertIn("robot-r8-evidence-${GITHUB_REF_NAME}.tar.gz", self.workflow)

    def test_convergence_gate_runs_before_release_job(self) -> None:
        self.assertIn("needs: [build, convergence-gate]", self.workflow)

    # -- immutable artifact property --------------------------------------

    def test_release_job_downloads_artifacts_and_publishes_without_rebuild(self) -> None:
        self.assertIn("actions/download-artifact@v4", self.workflow)
        self.assertIn("softprops/action-gh-release@v2", self.workflow)
        # The release job must not invoke cargo build
        release_marker = "\n  release:\n    name: Create Release"
        smoke_marker = "\n  smoke:"
        release_start = self.workflow.index(release_marker)
        release_end = self.workflow.index(smoke_marker, release_start)
        release_section = self.workflow[release_start:release_end]
        self.assertNotIn("cargo build", release_section)
        self.assertNotIn("cargo-agent.sh +stable build", release_section)

    # -- release notes ----------------------------------------------------

    def test_release_notes_include_commit_and_acceptance_run_id(self) -> None:
        self.assertIn("**Commit**:", self.workflow)
        self.assertIn("**Acceptance Run ID**:", self.workflow)
        self.assertIn("**R8 Positive Receipt Digest**:", self.workflow)
        self.assertIn("**R8 Negative Receipt Digest**:", self.workflow)

    def test_release_notes_require_not_optionally_omit_acceptance_ids(self) -> None:
        # The release notes step must fail if acceptance metadata is missing
        # or if any required field is empty.
        self.assertIn("acceptance metadata file is required but missing", self.workflow)
        self.assertIn("acceptance_run_id is required", self.workflow)
        self.assertIn("installed_digest is required", self.workflow)
        self.assertIn("R8 positive and negative receipt digests are required", self.workflow)

    def test_release_notes_run_git_from_workspace(self) -> None:
        # Release notes must run git from GITHUB_WORKSPACE, not RUNNER_TEMP.
        self.assertIn('cd "$GITHUB_WORKSPACE"', self.workflow)

    def test_release_notes_handle_first_release(self) -> None:
        # When PREV_TAG is empty, the compare URL must be omitted gracefully.
        self.assertIn('COMPARE_URL=""', self.workflow)
        self.assertIn("first tagged release", self.workflow)

    def test_release_notes_include_artifact_checksums(self) -> None:
        self.assertIn("### Artifacts", self.workflow)
        self.assertIn("SHA256:", self.workflow)

    def test_release_notes_include_known_limitations(self) -> None:
        self.assertIn("### Known Limitations", self.workflow)
        self.assertIn("Physical HIL (H1)", self.workflow)
        self.assertIn("Robot R8 Acceptance", self.workflow)

    def test_release_notes_include_rollback_point(self) -> None:
        self.assertIn("### Rollback", self.workflow)
        self.assertIn("migration-matrix.toml", self.workflow)

    def test_release_notes_include_robot_r8_consumed_evidence(self) -> None:
        self.assertIn(
            "Simulation Robot acceptance (positive completed + negative safe-stop) has been verified",
            self.workflow,
        )
        self.assertNotIn(
            "external operator-provided gate; it is not embedded in this automated workflow",
            self.workflow,
        )

    # -- post-release smoke job -------------------------------------------

    def test_smoke_job_exists(self) -> None:
        self.assertIn("smoke:", self.workflow)
        self.assertIn("Post-release smoke (x86_64)", self.workflow)

    def test_smoke_job_needs_release(self) -> None:
        # The smoke job must run after the release job publishes assets
        smoke_marker = "\n  smoke:\n    name: Post-release smoke"
        smoke_start = self.workflow.index(smoke_marker)
        # Look at the first 3 lines of the smoke job for needs:
        smoke_header = "\n".join(self.workflow[smoke_start:].split("\n")[:8])
        self.assertIn("needs: release", smoke_header)

    def test_smoke_verifies_exact_x86_checksum_line(self) -> None:
        # Must extract the exact SHA256SUMS line, not use --ignore-missing.
        self.assertIn('grep -F "${x86_basename}" SHA256SUMS', self.workflow)
        self.assertIn("sha256sum --check --strict", self.workflow)
        self.assertNotIn("--ignore-missing", self.workflow)

    def test_smoke_fails_on_missing_checksum_line(self) -> None:
        self.assertIn("not found in SHA256SUMS", self.workflow)

    def test_smoke_fails_on_duplicate_checksum_line(self) -> None:
        self.assertIn("multiple SHA256SUMS entries", self.workflow)

    def test_smoke_verifies_version_json_commit_binding(self) -> None:
        self.assertIn("Verify version JSON commit binding", self.workflow)
        self.assertIn('version JSON commit ${version_commit} != GITHUB_SHA', self.workflow)

    def test_smoke_downloads_published_x86_asset(self) -> None:
        self.assertIn("gh release download", self.workflow)
        self.assertIn("x86_64-unknown-linux-gnu.tar.gz", self.workflow)

    def test_smoke_verifies_checksums(self) -> None:
        self.assertIn("sha256sum --check --strict", self.workflow)
        self.assertNotIn("--ignore-missing", self.workflow)

    def test_smoke_runs_aletheon_version_json(self) -> None:
        self.assertIn('aletheon version --json', self.workflow)

    def test_smoke_writes_machine_readable_receipt(self) -> None:
        self.assertIn("smoke-receipt.json", self.workflow)
        self.assertIn('"smoke_checks"', self.workflow)
        self.assertIn('"rollback_point"', self.workflow)
        self.assertIn("r8_positive_receipt_sha256", self.workflow)
        self.assertIn("r8_negative_receipt_sha256", self.workflow)

    def test_smoke_receipt_has_actual_previous_tag(self) -> None:
        # The smoke receipt must compute the actual previous tag, not a
        # generic string.
        self.assertIn('PREV_TAG=$(git tag --sort=-v:refname', self.workflow)

    def test_smoke_fails_closed_on_version_failure(self) -> None:
        # The version check step exits non-zero on failure
        self.assertIn("aletheon version --json failed", self.workflow)

    def test_smoke_uploads_receipt_artifact(self) -> None:
        self.assertIn("smoke-receipt-${{ github.ref_name }}", self.workflow)

    def test_smoke_has_checkout_with_fetch_depth_zero(self) -> None:
        smoke_marker = "\n  smoke:\n    name: Post-release smoke"
        smoke_start = self.workflow.index(smoke_marker)
        smoke_header = self.workflow[smoke_start:smoke_start + 500]
        self.assertIn("actions/checkout@v4", smoke_header)
        self.assertIn("fetch-depth: 0", smoke_header)

    def test_smoke_missing_commit_is_hard_error(self) -> None:
        # Missing/empty commit must exit 1, not warn
        self.assertIn("version JSON does not include commit field", self.workflow)
        self.assertNotIn("WARNING: version JSON does not include commit", self.workflow)

    def test_smoke_safely_extracts_exactly_one_regular_aletheon(self) -> None:
        # Must use Python tarfile extraction, not tar xzf/find ambiguity
        self.assertIn("os.O_WRONLY | os.O_CREAT | os.O_EXCL", self.workflow)
        self.assertIn("exactly one regular aletheon binary", self.workflow)
        self.assertNotIn("tar xzf", self.workflow[self.workflow.index("Extract and smoke-test binary"):])

    def test_release_job_schema_checks_acceptance_metadata(self) -> None:
        # Release job must schema-check acceptance metadata and
        # recompute/bind all digests
        release_marker = "\n  release:\n    name: Create Release"
        smoke_marker = "\n  smoke:"
        release_start = self.workflow.index(release_marker)
        release_end = self.workflow.index(smoke_marker, release_start)
        release_job = self.workflow[release_start:release_end]
        self.assertIn("schema_version", release_job)
        self.assertIn("rc_archive_sha256 mismatch", release_job)
        self.assertIn("acceptance_archive_sha256 mismatch", release_job)
        self.assertIn("r8_evidence_archive_sha256 mismatch", release_job)

    def test_release_job_bundles_r8_evidence_archive(self) -> None:
        release_marker = "\n  release:\n    name: Create Release"
        smoke_marker = "\n  smoke:"
        release_start = self.workflow.index(release_marker)
        release_end = self.workflow.index(smoke_marker, release_start)
        release_job = self.workflow[release_start:release_end]
        self.assertIn("robot-r8-evidence-", release_job)
        self.assertIn("r8_evidence_archive_sha256", release_job)

    def test_smoke_validates_acceptance_metadata_commit_and_digests(self) -> None:
        # Must cross-check commit, installed_digest from metadata
        smoke_marker = "\n  smoke:\n    name: Post-release smoke"
        smoke_start = self.workflow.index(smoke_marker)
        smoke_job = self.workflow[smoke_start:]
        self.assertIn("acceptance metadata commit", smoke_job)
        self.assertIn("acceptance metadata installed_digest", smoke_job)
        self.assertIn("smoke binary digest", smoke_job)

    # -- fail-closed gate -------------------------------------------------

    def test_fail_closed_no_manual_truth_in_gate(self) -> None:
        # The gate must never trust the tag name / run-dir name as proof.
        # release_acceptance_gate.py internally calls verify_run() as the
        # single source of truth.
        self.assertIn(
            "release_acceptance_gate.py",
            self.workflow,
        )

    # -- no rebuild after acceptance --------------------------------------

    def test_immutable_artifact_no_rebuild_after_acceptance(self) -> None:
        # The release job downloads pre-built artifacts; it must never
        # invoke cargo build or any compilation step.
        # Extract the release job body between its header and the smoke job
        release_marker = "\n  release:\n    name: Create Release"
        smoke_marker = "\n  smoke:"
        release_start = self.workflow.index(release_marker)
        release_end = self.workflow.index(smoke_marker, release_start)
        release_job = self.workflow[release_start:release_end]
        self.assertNotIn("cargo build", release_job)
        self.assertNotIn("cargo-agent.sh", release_job)

    def test_smoke_no_rebuild(self) -> None:
        # The smoke job downloads the published release asset; it must
        # never invoke cargo build.
        smoke_marker = "\n  smoke:\n    name: Post-release smoke"
        smoke_start = self.workflow.index(smoke_marker)
        smoke_job = self.workflow[smoke_start:]
        self.assertNotIn("cargo build", smoke_job)
        self.assertNotIn("cargo-agent.sh", smoke_job)
        self.assertNotIn("rust-toolchain", smoke_job)


class R8EvidenceHandoffWorkflowTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.workflow = (ROOT / ".github/workflows/r8-evidence-handoff.yml").read_text()

    def test_two_jobs_declared(self) -> None:
        self.assertIn("stage-rc:", self.workflow)
        self.assertIn("validate-operator-produced-live-evidence:", self.workflow)

    def test_protected_environment_declared(self) -> None:
        marker = "validate-operator-produced-live-evidence:"
        protected_job_start = self.workflow.index(marker)
        protected_header = self.workflow[protected_job_start:protected_job_start + 400]
        self.assertIn("environment: robot-r8-release", protected_header)

    def test_no_literal_path_to_placeholders(self) -> None:
        self.assertNotIn("/path/to/", self.workflow)
        self.assertNotIn("/var/lib/aletheon/robot-episodes.db", self.workflow)

    def test_both_jobs_download_the_exact_rc_archive(self) -> None:
        self.assertEqual(self.workflow.count("actions/download-artifact@v4"), 2)
        self.assertEqual(self.workflow.count("run-id: ${{ inputs.rc_archive_run_id }}"), 2)
        self.assertEqual(
            self.workflow.count("pattern: aletheon-*-x86_64-unknown-linux-gnu.tar.gz"),
            2,
        )

    def test_protected_job_binds_environment_variable_names_exactly(self) -> None:
        # All evidence paths come from GitHub Environment vars (shell
        # variable references), not from workflow inputs or literals.
        required = [
            "ALETHEON_R8_POSITIVE_REPORT",
            "ALETHEON_R8_NEGATIVE_REPORT",
            "ALETHEON_R8_DATABASE",
            "ALETHEON_R8_EXPECTED_DEVICE",
            "ALETHEON_R8_EXPECTED_SCENE",
            "ALETHEON_R8_EXPECTED_BRIDGE_DIGEST",
            "ALETHEON_R8_EXPECTED_SKILL_DIGEST",
            "ALETHEON_R8_EXPECTED_POLICY_PROVIDER",
            "ALETHEON_R8_EXPECTED_POLICY_MODEL",
            "ALETHEON_R8_EXPECTED_POLICY_VERSION",
            "ALETHEON_R8_EXPECTED_POLICY_PROTOCOL",
            "ALETHEON_R8_EXPECTED_POLICY_DIGEST",
        ]
        for name in required:
            self.assertIn(f"${{{name}:?}}", self.workflow)

    def test_protected_job_calls_acceptance_robot_r8_twice(self) -> None:
        self.assertEqual(
            self.workflow.count("scripts/aletheon.sh acceptance robot-r8"), 2
        )
        self.assertIn("--require-safe-stop", self.workflow)

    def test_evidence_uploaded_only_on_success(self) -> None:
        # Evidence upload must use if: success(), not if: always()
        marker = "Upload R8 evidence artifact"
        upload_start = self.workflow.index(marker)
        upload_section = self.workflow[upload_start:upload_start + 300]
        self.assertIn("if: success()", upload_section)
        self.assertNotIn("if: always()", upload_section)

    def test_metadata_includes_all_required_fields(self) -> None:
        metadata_section = self.workflow[
            self.workflow.index("Bind evidence metadata"):
        ]
        self.assertIn("rc_archive_run_id", metadata_section)
        self.assertIn("rc_archive_sha256", metadata_section)
        self.assertIn("positive_receipt_sha256", metadata_section)
        self.assertIn("negative_receipt_sha256", metadata_section)
        self.assertIn("generated_utc", metadata_section)
        self.assertIn("datetime.timezone.utc", metadata_section)

    def test_validate_operator_job_calls_acceptance_not_execute_tasks(self) -> None:
        # The job name must say "validate", not "execute"
        self.assertIn("Validate operator-produced live evidence", self.workflow)
        self.assertNotIn("execute tasks", self.workflow.lower())


if __name__ == "__main__":
    unittest.main()
