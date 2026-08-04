#!/usr/bin/env python3
"""Exercise one real governed review over an installed Aletheon user socket."""

import hashlib
import json
import socket
import sys
import time
import uuid


def rpc(socket_path: str, method: str, params: dict, request_id: int) -> dict:
    request = {
        "jsonrpc": "2.0",
        "id": request_id,
        "method": method,
        "params": params,
    }
    with socket.socket(socket.AF_UNIX) as connection:
        connection.settimeout(150)
        connection.connect(socket_path)
        connection.sendall(json.dumps(request, separators=(",", ":")).encode() + b"\n")
        buffered = b""
        while True:
            chunk = connection.recv(65536)
            if not chunk:
                raise RuntimeError(f"socket closed before {method} response")
            buffered += chunk
            while b"\n" in buffered:
                line, buffered = buffered.split(b"\n", 1)
                response = json.loads(line)
                if response.get("id") == request_id:
                    if "error" in response:
                        raise RuntimeError(f"{method} failed: {response['error']}")
                    return response["result"]


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit("usage: governed-review-smoke.py SOCKET")
    socket_path = sys.argv[1]
    capabilities = rpc(socket_path, "review.capabilities", {}, 1)
    if capabilities.get("enabled") is not True:
        raise RuntimeError("governed review is not enabled")
    if capabilities.get("review", {}).get("schema_versions") != [2, 1]:
        raise RuntimeError("review schema negotiation drifted")

    content = "The supplied record states that its lifecycle status is active."
    evidence_digest = hashlib.sha256(content.encode()).hexdigest()
    job_id = f"release-smoke-{uuid.uuid4()}"
    job = {
        "schema_version": 2,
        "job_id": job_id,
        "subject_type": "record",
        "subject_refs": ["record/release-smoke"],
        "evidence": [
            {
                "reference": "evidence/release-smoke",
                "media_type": "text/plain",
                "content": content,
                "digest": evidence_digest,
                "observed_version": "v1",
            }
        ],
        "evidence_digest": evidence_digest,
        "policy_ref": "policy/evidence-only",
        "allowed_operations": ["retain"],
        "budget": {
            "max_input_bytes": 16384,
            "max_output_bytes": 8192,
            "max_tokens": 4096,
            "max_tool_calls": 0,
            "wall_time_ms": 120000,
        },
        "deadline_unix_ms": int(time.time() * 1000) + 120000,
        "idempotency_key": f"release-smoke-{uuid.uuid4()}",
    }
    submitted = rpc(socket_path, "review.submit", {"job": job}, 2)
    if submitted.get("job_id") != job_id or submitted.get("status") != "queued":
        raise RuntimeError(f"submit did not durably queue the job: {submitted}")
    waited = rpc(
        socket_path,
        "review.wait",
        {"job_id": job_id, "timeout_ms": 120000},
        3,
    )
    receipt = waited.get("receipt", {})
    if receipt.get("status") != "completed":
        raise RuntimeError(f"review did not complete: {receipt}")
    if receipt.get("job_id") != job_id or receipt.get("evidence_digest") != evidence_digest:
        raise RuntimeError("terminal receipt is not bound to submitted evidence")
    if receipt.get("usage", {}).get("inference_requests") != 1:
        raise RuntimeError("terminal receipt does not prove exactly one inference request")
    print(
        json.dumps(
            {
                "schema_version": 1,
                "status": "PASS",
                "socket": socket_path,
                "capabilities": capabilities,
                "receipt": receipt,
            },
            sort_keys=True,
            separators=(",", ":"),
        )
    )


if __name__ == "__main__":
    main()
