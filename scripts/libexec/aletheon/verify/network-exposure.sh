#!/usr/bin/env bash
set -euo pipefail

mode=${1:---strict}
case "$mode" in --strict|--snapshot) ;; *) echo "usage: $0 [--strict|--snapshot]" >&2; exit 64 ;; esac

failures=0
fail() { echo "FAIL: $*" >&2; failures=$((failures + 1)); }
note() { echo "INFO: $*" >&2; }

command -v ss >/dev/null || { echo "missing command: ss" >&2; exit 1; }
listeners=$(ss -H -lntup 2>/dev/null || true)

# Aletheon has no TCP administration endpoint. Any process named aletheon or
# GBrain container proxy must be loopback-only; the daemon itself uses AF_UNIX.
if awk '$0 ~ /(aletheon|gbrain|docker-proxy|rootlessport)/ && $5 !~ /^(127\.0\.0\.1|\[::1\]):/ {found=1} END {exit !found}' <<<"$listeners"; then
  fail "Aletheon-related process has a non-loopback TCP/UDP listener"
fi
if [[ -S /run/aletheon/aletheon.sock ]]; then
  mode_bits=$(stat -Lc '%a' /run/aletheon/aletheon.sock)
  (( (8#$mode_bits & 0007) == 0 )) || fail "daemon socket is accessible to other users"
  note "local daemon socket present with mode $mode_bits"
else
  [[ "$mode" == --snapshot ]] || fail "daemon Unix socket is absent"
  note "daemon Unix socket is absent"
fi

if command -v tailscale >/dev/null; then
  status_file=$(mktemp)
  trap 'rm -f -- "${status_file:-}"' EXIT
  if tailscale status --json >"$status_file" 2>/dev/null; then
    backend=$(jq -r '.BackendState // "unknown"' "$status_file" 2>/dev/null || echo unknown)
    [[ "$backend" == Running ]] || fail "Tailscale backend is not running"
    note "Tailscale backend state: $backend"
  else
    fail "cannot read Tailscale status"
  fi
  rm -f -- "$status_file"
  trap - EXIT
else
  [[ "$mode" == --snapshot ]] || fail "Tailscale CLI is unavailable"
  note "Tailscale CLI is unavailable"
fi

if command -v nft >/dev/null; then
  rules=$(nft list ruleset 2>/dev/null || true)
  grep -qE 'hook input.*policy drop|policy drop.*hook input' <<<"$rules" || fail "nftables input policy is not drop"
  grep -q 'iifname "tailscale0"' <<<"$rules" || fail "nftables has no tailscale0 administration rule"
elif command -v ufw >/dev/null; then
  rules=$(ufw status verbose 2>/dev/null || true)
  grep -q 'Default: deny (incoming)' <<<"$rules" || fail "UFW incoming policy is not deny"
  grep -qE '22/tcp.*ALLOW IN.*tailscale0|22/tcp on tailscale0' <<<"$rules" || fail "UFW has no tailscale0 SSH rule"
else
  [[ "$mode" == --snapshot ]] || fail "no supported host firewall inspector is available"
  note "no supported host firewall inspector is available"
fi

((failures == 0)) || exit 1
echo "network exposure verification passed" >&2
