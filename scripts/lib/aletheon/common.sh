#!/usr/bin/env bash
# Shared helpers for scripts/aletheon.sh. Source; do not execute.

_aletheon_ts() { date -u +%Y-%m-%dT%H:%M:%SZ; }
aletheon_info() { printf '[%s] [INFO] %s\n' "$(_aletheon_ts)" "$*"; }
aletheon_ok() { printf '[%s] [OK] %s\n' "$(_aletheon_ts)" "$*"; }
aletheon_warn() { printf '[%s] [WARN] %s\n' "$(_aletheon_ts)" "$*" >&2; }
aletheon_die() { printf '[%s] [ERROR] %s\n' "$(_aletheon_ts)" "$*" >&2; return 1; }

require_command() { command -v "$1" >/dev/null 2>&1 || aletheon_die "required command is unavailable: $1"; }

ALETHEON_CONFIG_FILE=${ALETHEON_CONFIG_FILE:-$HOME/.aletheon/config.toml}
ALETHEON_USER_SOCKET=${ALETHEON_USER_SOCKET:-${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/aletheon/aletheon.sock}
ALETHEON_CORE_SOCKET=${ALETHEON_CORE_SOCKET:-/run/aletheon/core.sock}
ALETHEON_RELEASE_BINARY=${ALETHEON_RELEASE_BINARY:-$ALETHEON_ROOT/target/release/aletheon}

validate_http_endpoint() {
  python3 - "$1" <<'PY'
import sys
from urllib.parse import urlparse
value = sys.argv[1]
parsed = urlparse(value)
if parsed.scheme not in {"http", "https"} or not parsed.hostname or parsed.username or parsed.password:
    raise SystemExit(f"invalid HTTP endpoint: {value}")
PY
}

gbrain_endpoint() {
  [[ -f "$ALETHEON_CONFIG_FILE" ]] || return 1
  python3 - "$ALETHEON_CONFIG_FILE" <<'PY'
import sys, tomllib
with open(sys.argv[1], "rb") as source:
    config = tomllib.load(source)
for server in config.get("mcp_servers", []):
    if server.get("name") == "gbrain" and server.get("url"):
        print(server["url"])
        raise SystemExit(0)
raise SystemExit(1)
PY
}

gbrain_health_url() {
  python3 - "$1" <<'PY'
import sys
from urllib.parse import urlparse, urlunparse
p = urlparse(sys.argv[1])
print(urlunparse((p.scheme, p.netloc, "/health", "", "", "")))
PY
}
