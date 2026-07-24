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
ALETHEON_INSTALLED_BINARY=${ALETHEON_INSTALLED_BINARY:-/usr/bin/aletheon}
ALETHEON_PROC_ROOT=${ALETHEON_PROC_ROOT:-/proc}
ALETHEON_CORE_UNIT=${ALETHEON_CORE_UNIT:-aletheon-core.service}
ALETHEON_USER_UNIT=${ALETHEON_USER_UNIT:-aletheon.service}
ALETHEON_STABILITY_SECONDS=${ALETHEON_STABILITY_SECONDS:-7}
ALETHEON_SMOKE_TIMEOUT_SECONDS=${ALETHEON_SMOKE_TIMEOUT_SECONDS:-60}
ALETHEON_SMOKE_PROMPT=${ALETHEON_SMOKE_PROMPT:-Reply with exactly: ALETHEON_DEPLOYMENT_OK}
ALETHEON_LIBEXEC=${ALETHEON_LIBEXEC:-$ALETHEON_ROOT/scripts/libexec/aletheon}

run_internal() {
  local relative=$1
  shift
  local command=$ALETHEON_LIBEXEC/$relative
  [[ -x "$command" ]] || {
    aletheon_die "internal command is unavailable: $relative"
    return
  }
  "$command" "$@"
}

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
