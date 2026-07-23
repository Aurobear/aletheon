#!/usr/bin/env python3
"""Deterministic hostile/healthy JSON-RPC fixture for extension runtime gates."""

import json
import os
import socket
import sys
import time


MODE = os.environ.get("EXTENSION_FIXTURE_MODE", "normal")


def respond(request, result=None, *, response_id=None, version="2.0"):
    payload = {
        "jsonrpc": version,
        "id": request["id"] if response_id is None else response_id,
        "result": result,
    }
    sys.stdout.write(json.dumps(payload, separators=(",", ":")) + "\n")
    sys.stdout.flush()


for raw in sys.stdin:
    request = json.loads(raw)
    method = request.get("method")
    if method == "initialize":
        respond(request, {"ready": True})
    elif method == "shutdown":
        respond(request, {"stopped": True})
        break
    elif method == "start" and MODE == "business_fail":
        params = request["params"]
        respond(
            request,
            {
                "agent_id": params["root_agent_id"],
                "root_agent_id": params["root_agent_id"],
                "parent_agent_id": params.get("parent_agent_id"),
                "process_id": "22222222-2222-4222-8222-222222222222",
                "operation_id": "33333333-3333-4333-8333-333333333333",
                "runtime_id": params["runtime_id"],
                "profile_id": params["profile_id"],
            },
        )
    elif MODE == "business_fail":
        respond(request, {}, response_id=request["id"] + 1)
    elif MODE == "wrong_id":
        respond(request, {}, response_id=request["id"] + 1)
    elif MODE == "wrong_version":
        respond(request, {}, version="1.0")
    elif MODE == "oversized":
        respond(request, {"output": "x" * (11 * 1024 * 1024)})
    elif MODE == "hang":
        time.sleep(60)
    elif MODE == "crash":
        os._exit(23)
    elif MODE == "stderr_secret":
        sys.stderr.write("api_key=fixture-must-be-redacted\n")
        sys.stderr.flush()
        respond(request, {"ok": True})
    elif MODE == "filesystem_probe":
        target = os.environ.get("EXTENSION_FORBIDDEN_PATH", "/root/.ssh/id_rsa")
        try:
            with open(target, "rb"):
                allowed = True
        except OSError:
            allowed = False
        respond(request, {"allowed": allowed})
    elif MODE == "network_probe":
        sock = socket.socket()
        try:
            sock.settimeout(0.2)
            sock.connect(("127.0.0.1", 9))
            allowed = True
        except OSError:
            allowed = False
        finally:
            sock.close()
        respond(request, {"allowed": allowed})
    else:
        respond(request, {"method": method, "ok": True})
