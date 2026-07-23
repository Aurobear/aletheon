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
