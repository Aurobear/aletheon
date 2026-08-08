#!/usr/bin/env python3
"""R8 evidence verifier: validate machine-readable positive and negative
Robot R8 acceptance receipts against the release candidate.

Called by the release workflow convergence-gate job. Fails closed.

Requires the R8 metadata file produced by the evidence handoff workflow
alongside the two receipt files.  The verifier:

  * validates metadata schema, commit, run ID, and device;
  * recomputes the actual RC archive digest and compares to metadata;
  * recomputes both receipt file digests and compares to metadata;
  * compares metadata installed_digest to the expected installed digest;
  * validates the positive receipt shape (schema 1, REPORT_EVIDENCE_PASS,
    completed, at least one attempt, operation_ids count matches
    attempt_count, safe_stop is None, runtime facts present);
  * validates the negative receipt shape (schema 1, REPORT_EVIDENCE_PASS,
    failed, safe_stop object with outcome=succeeded,
    attempted_after_attempt equal to attempt_count, no invented
    safe_stop.triggered requirement, operation_ids count matches
    attempt_count with zero attempts valid);
  * ensures positive and negative episode IDs and report digests differ;
  * validates metadata generated_utc is a parseable timezone-aware UTC
    timestamp;
  * freshness is bound by rc_archive_run_id matching the current release
    GITHUB_RUN_ID.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from datetime import datetime, timezone
from pathlib import Path


# ---------------------------------------------------------------------------
# helpers
# ---------------------------------------------------------------------------

def _is_lowercase_hex(value: str, length: int) -> bool:
    """Return True if *value* is exactly *length* lowercase hex chars."""
    if len(value) != length:
        return False
    return all(c in "0123456789abcdef" for c in value)


def _require_safe_regular_file(path: Path, label: str) -> list[str]:
    """Check *path* is a regular file, not a symlink, and readable."""
    reasons: list[str] = []
    if not path.exists():
        reasons.append(f"{label} does not exist: {path}")
        return reasons
    if path.is_symlink():
        reasons.append(f"{label} is a symlink; regular file required: {path}")
    if not path.is_file():
        reasons.append(f"{label} is not a regular file: {path}")
    return reasons


def _reject_json_constant(value: str) -> None:
    raise ValueError(f"non-finite JSON constant is not accepted: {value}")


def _load_json_file(path: Path, label: str) -> tuple[dict | None, list[str]]:
    """Load and parse a JSON file; returns (parsed_dict, reasons).

    Rejects NaN/Infinity via parse_constant.
    """
    reasons = _require_safe_regular_file(path, label)
    if reasons:
        return None, reasons
    try:
        raw = path.read_text(encoding="utf-8")
    except OSError as exc:
        return None, [f"cannot read {label}: {exc}"]
    try:
        data = json.loads(raw, parse_constant=_reject_json_constant)
    except (json.JSONDecodeError, ValueError) as exc:
        return None, [f"cannot parse {label}: {exc}"]
    if not isinstance(data, dict):
        return None, [f"{label} is not a JSON object"]
    return data, []


# ---------------------------------------------------------------------------
# metadata validation
# ---------------------------------------------------------------------------

def verify_metadata(
    metadata: dict,
    expected_commit: str,
    expected_run_id: str,
    expected_rc_archive_path: Path,
    expected_installed_digest: str,
    expected_device: str,
    positive_path: Path,
    negative_path: Path,
) -> list[str]:
    """Validate the R8 metadata file and cross-check digests."""
    reasons: list[str] = []

    if metadata.get("schema_version") != 1:
        reasons.append("metadata schema_version must be 1")

    commit_sha = metadata.get("commit_sha")
    if not isinstance(commit_sha, str) or not _is_lowercase_hex(commit_sha, 40):
        reasons.append("metadata commit_sha must be a 40-char lowercase hex string")
    elif commit_sha != expected_commit:
        reasons.append(
            f"metadata commit_sha {commit_sha} != expected {expected_commit}"
        )

    rc_run_id = metadata.get("rc_archive_run_id")
    if not isinstance(rc_run_id, str) or not rc_run_id:
        reasons.append("metadata rc_archive_run_id must be a non-empty string")
    elif rc_run_id != expected_run_id:
        reasons.append(
            f"metadata rc_archive_run_id {rc_run_id} != expected "
            f"{expected_run_id}"
        )

    if metadata.get("device") != expected_device:
        reasons.append(
            f"metadata device mismatch: "
            f"expected={expected_device!r}"
        )

    # generated_utc must be a parseable UTC timestamp with offset exactly zero
    generated_dt: datetime | None = None
    generated_utc = metadata.get("generated_utc")
    if not isinstance(generated_utc, str):
        reasons.append("metadata generated_utc must be a string")
    else:
        try:
            dt = datetime.fromisoformat(generated_utc)
        except (ValueError, TypeError):
            reasons.append(
                f"metadata generated_utc is not a valid ISO timestamp: "
                f"{generated_utc!r}"
            )
        else:
            if dt.tzinfo is None:
                reasons.append(
                    "metadata generated_utc must be timezone-aware (UTC)"
                )
            elif dt.utcoffset() != timezone.utc.utcoffset(None):
                reasons.append(
                    "metadata generated_utc UTC offset must be exactly zero"
                )
            else:
                generated_dt = dt

    # stage_completed_utc must be present, parseable, UTC offset exactly zero
    stage_dt: datetime | None = None
    stage_completed_utc = metadata.get("stage_completed_utc")
    if not isinstance(stage_completed_utc, str):
        reasons.append("metadata stage_completed_utc must be a non-empty string")
    else:
        try:
            dt_stage = datetime.fromisoformat(stage_completed_utc)
        except (ValueError, TypeError):
            reasons.append(
                f"metadata stage_completed_utc is not a valid ISO timestamp: "
                f"{stage_completed_utc!r}"
            )
        else:
            if dt_stage.tzinfo is None:
                reasons.append(
                    "metadata stage_completed_utc must be timezone-aware (UTC)"
                )
            elif dt_stage.utcoffset() != timezone.utc.utcoffset(None):
                reasons.append(
                    "metadata stage_completed_utc UTC offset must be exactly zero"
                )
            else:
                stage_dt = dt_stage

    if (
        generated_dt is not None
        and stage_dt is not None
        and generated_dt < stage_dt
    ):
        reasons.append("metadata generated_utc must not precede stage_completed_utc")

    # stage_completed_unix_ms must be a non-negative integer
    stage_completed_unix_ms = metadata.get("stage_completed_unix_ms")
    if (
        not isinstance(stage_completed_unix_ms, int)
        or isinstance(stage_completed_unix_ms, bool)
        or stage_completed_unix_ms < 0
    ):
        reasons.append("metadata stage_completed_unix_ms must be a non-negative integer")
    elif stage_dt is not None:
        stage_utc_unix_ms = int(stage_dt.timestamp() * 1000)
        if stage_completed_unix_ms != stage_utc_unix_ms:
            reasons.append(
                "metadata stage_completed_utc and stage_completed_unix_ms "
                "must describe the same instant"
            )

    # Settlement timestamps must be non-negative and no older than the stage.
    for field in (
        "positive_settled_at_unix_ms",
        "negative_settled_at_unix_ms",
    ):
        val = metadata.get(field)
        if not isinstance(val, int) or isinstance(val, bool) or val < 0:
            reasons.append(f"metadata {field} must be a non-negative integer")
        elif (
            isinstance(stage_completed_unix_ms, int)
            and not isinstance(stage_completed_unix_ms, bool)
            and val < stage_completed_unix_ms
        ):
            reasons.append(
                f"metadata {field}={val} is before "
                f"stage_completed_unix_ms={stage_completed_unix_ms}"
            )

    # positive/negative episode IDs must differ (embedded in metadata for freshness)
    pos_ep_id_meta = metadata.get("positive_episode_id")
    neg_ep_id_meta = metadata.get("negative_episode_id")
    if not isinstance(pos_ep_id_meta, str) or not pos_ep_id_meta.strip():
        reasons.append("metadata positive_episode_id must be a non-empty string")
    if not isinstance(neg_ep_id_meta, str) or not neg_ep_id_meta.strip():
        reasons.append("metadata negative_episode_id must be a non-empty string")
    if (
        isinstance(pos_ep_id_meta, str)
        and isinstance(neg_ep_id_meta, str)
        and pos_ep_id_meta.strip() == neg_ep_id_meta.strip()
    ):
        reasons.append("metadata positive_episode_id and negative_episode_id must differ")

    # recompute actual RC archive digest
    rc_archive_sha256 = metadata.get("rc_archive_sha256")
    if not isinstance(rc_archive_sha256, str) or not _is_lowercase_hex(
        rc_archive_sha256, 64
    ):
        reasons.append("metadata rc_archive_sha256 must be a 64-char lowercase hex string")
    else:
        file_reasons = _require_safe_regular_file(expected_rc_archive_path, "RC archive")
        if file_reasons:
            reasons.extend(file_reasons)
        else:
            actual_archive_digest = hashlib.sha256(
                expected_rc_archive_path.read_bytes()
            ).hexdigest()
            if actual_archive_digest != rc_archive_sha256:
                reasons.append(
                    f"metadata rc_archive_sha256 {rc_archive_sha256} != "
                    f"actual archive digest {actual_archive_digest}"
                )

    # compare metadata installed_digest to expected current installed digest
    installed_digest = metadata.get("installed_digest")
    if not isinstance(installed_digest, str) or not _is_lowercase_hex(installed_digest, 64):
        reasons.append("metadata installed_digest must be a 64-char lowercase hex string")
    elif installed_digest != expected_installed_digest:
        reasons.append(
            f"metadata installed_digest {installed_digest} != "
            f"expected installed digest {expected_installed_digest}"
        )

    # recompute receipt file digests and compare metadata
    pos_digest_meta = metadata.get("positive_receipt_sha256")
    if not isinstance(pos_digest_meta, str) or not _is_lowercase_hex(pos_digest_meta, 64):
        reasons.append("metadata positive_receipt_sha256 must be a 64-char lowercase hex string")
    else:
        pos_reasons = _require_safe_regular_file(positive_path, "positive receipt")
        if pos_reasons:
            reasons.extend(pos_reasons)
        else:
            actual_pos_digest = hashlib.sha256(
                positive_path.read_bytes()
            ).hexdigest()
            if actual_pos_digest != pos_digest_meta:
                reasons.append(
                    f"metadata positive_receipt_sha256 {pos_digest_meta} != "
                    f"actual file digest {actual_pos_digest}"
                )

    neg_digest_meta = metadata.get("negative_receipt_sha256")
    if not isinstance(neg_digest_meta, str) or not _is_lowercase_hex(neg_digest_meta, 64):
        reasons.append("metadata negative_receipt_sha256 must be a 64-char lowercase hex string")
    else:
        neg_reasons = _require_safe_regular_file(negative_path, "negative receipt")
        if neg_reasons:
            reasons.extend(neg_reasons)
        else:
            actual_neg_digest = hashlib.sha256(
                negative_path.read_bytes()
            ).hexdigest()
            if actual_neg_digest != neg_digest_meta:
                reasons.append(
                    f"metadata negative_receipt_sha256 {neg_digest_meta} != "
                    f"actual file digest {actual_neg_digest}"
                )

    return reasons


# ---------------------------------------------------------------------------
# receipt shape validation
# ---------------------------------------------------------------------------

def _validate_runtime_facts(
    runtime_facts: dict, expected_device: str
) -> list[str]:
    """Validate runtime_facts block common to both receipts."""
    reasons: list[str] = []
    if runtime_facts.get("device") != expected_device:
        reasons.append(
            f"runtime_facts device {runtime_facts.get('device')!r} "
            f"!= expected {expected_device!r}"
        )
    scene = runtime_facts.get("scene")
    if not isinstance(scene, str) or not scene:
        reasons.append("runtime_facts scene must be a non-empty string")
    bridge = runtime_facts.get("bridge_protocol_digest")
    if not isinstance(bridge, str) or not _is_lowercase_hex(bridge, 64):
        reasons.append(
            "runtime_facts bridge_protocol_digest must be a 64-char "
            "lowercase hex string"
        )
    skill = runtime_facts.get("skill_descriptor_digest")
    if not isinstance(skill, str) or not _is_lowercase_hex(skill, 64):
        reasons.append(
            "runtime_facts skill_descriptor_digest must be a 64-char "
            "lowercase hex string"
        )
    policy = runtime_facts.get("policy")
    if not isinstance(policy, dict):
        reasons.append("runtime_facts policy must be a dict")
    else:
        for field in ("provider", "model", "version", "protocol_version", "digest"):
            val = policy.get(field)
            if not isinstance(val, str) or not val:
                reasons.append(
                    f"runtime_facts policy.{field} must be a non-empty string"
                )
        pol_digest = policy.get("digest", "")
        if not isinstance(pol_digest, str) or not _is_lowercase_hex(
            pol_digest, 64
        ):
            reasons.append(
                "runtime_facts policy.digest must be a 64-char lowercase hex string"
            )
    return reasons


def verify_positive_receipt(
    receipt: dict,
    expected_device: str,
) -> list[str]:
    """Return a list of rejection reasons (empty = valid)."""
    reasons: list[str] = []

    if not isinstance(receipt, dict):
        return ["positive receipt is not a JSON object"]

    if receipt.get("schema_version") != 1:
        reasons.append("positive receipt schema_version must be 1")

    if receipt.get("status") != "REPORT_EVIDENCE_PASS":
        reasons.append(
            f"positive receipt status must be REPORT_EVIDENCE_PASS,"
            f" got {receipt.get('status')!r}"
        )

    if receipt.get("settlement") != "completed":
        reasons.append(
            f"positive receipt settlement must be completed,"
            f" got {receipt.get('settlement')!r}"
        )

    # episode_id must be non-empty
    episode_id = receipt.get("episode_id")
    if not isinstance(episode_id, str) or not episode_id.strip():
        reasons.append("positive receipt episode_id must be a non-empty string")

    # at least one attempt (reject bool masquerading as int)
    attempt_count = receipt.get("attempt_count")
    if (
        not isinstance(attempt_count, int)
        or isinstance(attempt_count, bool)
        or attempt_count < 1
    ):
        reasons.append(
            "positive receipt attempt_count must be a positive integer"
        )

    operation_ids = receipt.get("operation_ids")
    if not isinstance(operation_ids, list) or len(operation_ids) == 0:
        reasons.append(
            "positive receipt operation_ids must be a non-empty list"
        )
    elif isinstance(attempt_count, int) and attempt_count >= 1:
        if len(operation_ids) != attempt_count:
            reasons.append(
                f"positive receipt operation_ids count {len(operation_ids)}"
                f" != attempt_count {attempt_count}"
            )
        # unique and nonempty
        operation_ids_are_strings = True
        for i, opid in enumerate(operation_ids):
            if not isinstance(opid, str) or not opid:
                operation_ids_are_strings = False
                reasons.append(
                    f"positive receipt operation_ids[{i}] must be a "
                    f"non-empty string"
                )
        if operation_ids_are_strings and len(set(operation_ids)) != len(
            operation_ids
        ):
            reasons.append(
                "positive receipt operation_ids must be unique"
            )

    report_sha256 = receipt.get("report_sha256")
    if not isinstance(report_sha256, str) or not _is_lowercase_hex(
        report_sha256, 64
    ):
        reasons.append(
            "positive receipt report_sha256 must be a 64-char lowercase hex string"
        )

    # safe_stop must be None for completed
    if receipt.get("safe_stop") is not None:
        reasons.append("positive receipt safe_stop must be None for completed settlement")

    runtime_facts = receipt.get("runtime_facts")
    if not isinstance(runtime_facts, dict):
        reasons.append("positive receipt runtime_facts must be a dict")
    else:
        reasons.extend(_validate_runtime_facts(runtime_facts, expected_device))

    return reasons


def verify_negative_receipt(
    receipt: dict,
    expected_device: str,
) -> list[str]:
    """Return a list of rejection reasons (empty = valid)."""
    reasons: list[str] = []

    if not isinstance(receipt, dict):
        return ["negative receipt is not a JSON object"]

    if receipt.get("schema_version") != 1:
        reasons.append("negative receipt schema_version must be 1")

    if receipt.get("status") != "REPORT_EVIDENCE_PASS":
        reasons.append(
            f"negative receipt status must be REPORT_EVIDENCE_PASS,"
            f" got {receipt.get('status')!r}"
        )

    if receipt.get("settlement") != "failed":
        reasons.append(
            f"negative receipt settlement must be failed,"
            f" got {receipt.get('settlement')!r}"
        )

    # episode_id must be non-empty
    episode_id = receipt.get("episode_id")
    if not isinstance(episode_id, str) or not episode_id.strip():
        reasons.append("negative receipt episode_id must be a non-empty string")

    attempt_count = receipt.get("attempt_count")
    if (
        not isinstance(attempt_count, int)
        or isinstance(attempt_count, bool)
        or attempt_count < 0
    ):
        reasons.append(
            "negative receipt attempt_count must be a non-negative integer"
        )

    operation_ids = receipt.get("operation_ids")
    if not isinstance(operation_ids, list):
        reasons.append("negative receipt operation_ids must be a list")
    elif isinstance(attempt_count, int):
        if len(operation_ids) != attempt_count:
            reasons.append(
                f"negative receipt operation_ids count {len(operation_ids)}"
                f" != attempt_count {attempt_count}"
            )
        operation_ids_are_strings = True
        for i, opid in enumerate(operation_ids):
            if not isinstance(opid, str) or not opid:
                operation_ids_are_strings = False
                reasons.append(
                    f"negative receipt operation_ids[{i}] must be a "
                    f"non-empty string"
                )
        if (
            operation_ids_are_strings
            and len(operation_ids) > 0
            and len(set(operation_ids)) != len(operation_ids)
        ):
            reasons.append(
                "negative receipt operation_ids must be unique when non-empty"
            )

    report_sha256 = receipt.get("report_sha256")
    if not isinstance(report_sha256, str) or not _is_lowercase_hex(
        report_sha256, 64
    ):
        reasons.append(
            "negative receipt report_sha256 must be a 64-char lowercase hex string"
        )

    safe_stop = receipt.get("safe_stop")
    if not isinstance(safe_stop, dict):
        reasons.append("negative receipt safe_stop must be a dict")
    else:
        # Authoritative SafeStopReceipt keys: exactly outcome,
        # attempted_after_attempt, and optional trigger. Reject
        # invented extra keys (e.g. triggered, status, etc.).
        allowed_keys = {"outcome", "attempted_after_attempt", "trigger"}
        extra_keys = set(safe_stop.keys()) - allowed_keys
        if extra_keys:
            reasons.append(
                "negative receipt safe_stop contains disallowed keys: "
                + ", ".join(sorted(extra_keys))
            )
        outcome = safe_stop.get("outcome")
        if outcome != "succeeded":
            reasons.append(
                f"negative receipt safe_stop.outcome must be succeeded,"
                f" got {outcome!r}"
            )
        # attempted_after_attempt must be a non-negative integer equal to
        # attempt_count (per authoritative robot_r8_evidence.py:268-270).
        # Reject bool masquerading as int.
        after_attempt = safe_stop.get("attempted_after_attempt")
        if (
            not isinstance(after_attempt, int)
            or isinstance(after_attempt, bool)
            or after_attempt < 0
        ):
            reasons.append(
                "negative receipt safe_stop.attempted_after_attempt must "
                "be a non-negative integer"
            )
        elif (
            isinstance(attempt_count, int)
            and not isinstance(attempt_count, bool)
            and after_attempt != attempt_count
        ):
            reasons.append(
                f"negative receipt safe_stop.attempted_after_attempt "
                f"{after_attempt} != attempt_count {attempt_count}"
            )
        # trigger is optional; if present it must be a non-empty string.
        if "trigger" in safe_stop:
            trigger = safe_stop["trigger"]
            if trigger is not None and (
                not isinstance(trigger, str) or not trigger
            ):
                reasons.append(
                    "negative receipt safe_stop.trigger, when present, "
                    "must be a non-empty string"
                )

    runtime_facts = receipt.get("runtime_facts")
    if not isinstance(runtime_facts, dict):
        reasons.append("negative receipt runtime_facts must be a dict")
    else:
        reasons.extend(_validate_runtime_facts(runtime_facts, expected_device))

    return reasons


def verify_distinct_episodes(
    positive: dict, negative: dict
) -> list[str]:
    """Ensure positive and negative episode IDs and report digests differ."""
    reasons: list[str] = []
    pos_id = positive.get("episode_id")
    neg_id = negative.get("episode_id")
    if pos_id == neg_id:
        reasons.append(
            f"positive and negative episode_id must differ: {pos_id!r}"
        )
    pos_digest = positive.get("report_sha256")
    neg_digest = negative.get("report_sha256")
    if pos_digest == neg_digest:
        reasons.append(
            "positive and negative report_sha256 must differ"
        )
    return reasons


def verify_metadata_receipt_binding(
    metadata: dict, positive: dict, negative: dict
) -> list[str]:
    """Bind freshness metadata to the exact receipt episode identifiers."""
    reasons: list[str] = []
    for label, receipt in (("positive", positive), ("negative", negative)):
        metadata_id = metadata.get(f"{label}_episode_id")
        receipt_id = receipt.get("episode_id")
        if metadata_id != receipt_id:
            reasons.append(
                f"metadata {label}_episode_id {metadata_id!r} does not match "
                f"the {label} receipt episode_id {receipt_id!r}"
            )
    return reasons


# ---------------------------------------------------------------------------
# top-level verifier
# ---------------------------------------------------------------------------

def run_verifier(
    metadata_path: Path,
    positive_path: Path,
    negative_path: Path,
    expected_commit: str,
    expected_run_id: str,
    expected_rc_archive_path: Path,
    expected_installed_digest: str,
    expected_device: str,
) -> int:
    """Execute the R8 evidence verification. Returns 0 on success, 1 on
    any violation.
    """
    errors: list[str] = []

    # --- metadata ---
    metadata, meta_errs = _load_json_file(metadata_path, "R8 metadata")
    if meta_errs:
        errors.extend(meta_errs)
    else:
        errors.extend(verify_metadata(
            metadata,
            expected_commit,
            expected_run_id,
            expected_rc_archive_path,
            expected_installed_digest,
            expected_device,
            positive_path,
            negative_path,
        ))

    if errors:
        print("R8 evidence verification FAILED:", file=sys.stderr)
        for err in errors:
            print(f"  - {err}", file=sys.stderr)
        return 1

    # --- positive receipt ---
    positive, pos_errs = _load_json_file(positive_path, "positive receipt")
    if pos_errs:
        errors.extend(pos_errs)
    else:
        errors.extend(verify_positive_receipt(positive, expected_device))

    # --- negative receipt ---
    negative, neg_errs = _load_json_file(negative_path, "negative receipt")
    if neg_errs:
        errors.extend(neg_errs)
    else:
        errors.extend(verify_negative_receipt(negative, expected_device))

    # --- cross-validation ---
    if positive is not None and negative is not None:
        errors.extend(verify_distinct_episodes(positive, negative))
        if metadata is not None:
            errors.extend(
                verify_metadata_receipt_binding(metadata, positive, negative)
            )

    if errors:
        print("R8 evidence verification FAILED:", file=sys.stderr)
        for err in errors:
            print(f"  - {err}", file=sys.stderr)
        return 1

    print("R8 evidence verification PASSED")
    print(f"  metadata:          {metadata_path}")
    print(f"  positive receipt:  {positive_path}")
    print(f"  negative receipt:  {negative_path}")
    print(f"  expected commit:   {expected_commit}")
    print(f"  expected run ID:   {expected_run_id}")
    print(f"  expected device:   {expected_device}")
    return 0


def main() -> None:
    parser = argparse.ArgumentParser(
        description="R8 evidence verifier CLI."
    )
    parser.add_argument(
        "--metadata", required=True,
        help="Path to r8-metadata.json from the evidence handoff workflow",
    )
    parser.add_argument(
        "--positive-receipt", required=True,
        help="Path to the positive (completed) R8 receipt JSON",
    )
    parser.add_argument(
        "--negative-receipt", required=True,
        help="Path to the negative (failed + safe-stop) R8 receipt JSON",
    )
    parser.add_argument(
        "--expected-commit", required=True,
        help="Expected GITHUB_SHA this evidence must be bound to",
    )
    parser.add_argument(
        "--expected-run-id", required=True,
        help="Expected GITHUB_RUN_ID (current release run) for freshness",
    )
    parser.add_argument(
        "--expected-rc-archive", required=True,
        help="Path to the exact x86_64 RC archive for digest cross-check",
    )
    parser.add_argument(
        "--expected-installed-digest", required=True,
        help="Expected sha256sum of /usr/bin/aletheon after RC deployment",
    )
    parser.add_argument(
        "--expected-device", required=True,
        help="Expected robot device identifier",
    )
    args = parser.parse_args()
    sys.exit(run_verifier(
        metadata_path=Path(args.metadata),
        positive_path=Path(args.positive_receipt),
        negative_path=Path(args.negative_receipt),
        expected_commit=args.expected_commit,
        expected_run_id=args.expected_run_id,
        expected_rc_archive_path=Path(args.expected_rc_archive),
        expected_installed_digest=args.expected_installed_digest,
        expected_device=args.expected_device,
    ))


if __name__ == "__main__":
    main()
