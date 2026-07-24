#!/usr/bin/env bash
# Installed production runtime verification. Source; do not execute.

_service_property() {
  local scope=$1 unit=$2 property=$3
  if [[ "$scope" == user ]]; then
    systemctl --user show "$unit" --property="$property" --value
  else
    systemctl show "$unit" --property="$property" --value
  fi
}

_service_pid() {
  local scope=$1 unit=$2 pid
  pid=$(_service_property "$scope" "$unit" MainPID)
  [[ "$pid" =~ ^[1-9][0-9]*$ ]] ||
    aletheon_die "$scope service has no running main process: $unit"
  printf '%s\n' "$pid"
}

_runtime_executable() {
  local scope=$1 unit=$2 pid path
  pid=$(_service_pid "$scope" "$unit") || return
  path=$(readlink -f "$ALETHEON_PROC_ROOT/$pid/exe" 2>/dev/null) || path=
  if [[ ! -x "$path" ]]; then
    path=$(_service_property "$scope" "$unit" ExecStart | sed -n 's/.*path=\([^ ;}]*\).*/\1/p')
    [[ -n "$path" ]] || {
      aletheon_die "cannot resolve $scope runtime executable for pid $pid"
      return
    }
    path=$(readlink -f "$path") || {
      aletheon_die "cannot resolve configured $scope runtime executable: $path"
      return
    }
  fi
  [[ -x "$path" ]] ||
    { aletheon_die "$scope runtime executable is unavailable: $path"; return; }
  printf '%s\n' "$path"
}

_sha256() {
  [[ -x "$1" ]] || { aletheon_die "required executable is unavailable: $1"; return; }
  sha256sum -- "$1" | awk '{print $1}'
}

cmd_verify_runtime_provenance() {
  local candidate installed installed_path core_path user_path
  local candidate_hash installed_hash core_hash user_hash
  candidate=$ALETHEON_RELEASE_BINARY
  installed=$ALETHEON_INSTALLED_BINARY
  installed_path=$(readlink -f "$installed") ||
    { aletheon_die "cannot resolve installed binary: $installed"; return; }
  core_path=$(_runtime_executable system "$ALETHEON_CORE_UNIT") || return
  user_path=$(_runtime_executable user "$ALETHEON_USER_UNIT") || return
  candidate_hash=$(_sha256 "$candidate") || return
  installed_hash=$(_sha256 "$installed") || return
  core_hash=$(_sha256 "$core_path") || return
  user_hash=$(_sha256 "$user_path") || return

  if [[ "$installed_hash" != "$candidate_hash" ]]; then
    aletheon_die "installed binary hash differs from release candidate: candidate=$candidate_hash installed=$installed_hash"
    return
  fi
  if [[ "$core_hash" != "$candidate_hash" || "$user_hash" != "$candidate_hash" ]]; then
    aletheon_die "running executable hash differs from release candidate: candidate=$candidate_hash core=$core_hash user=$user_hash"
    return
  fi
  if [[ "$core_path" != "$installed_path" || "$user_path" != "$installed_path" ]]; then
    aletheon_die "running executable path differs from installed binary: installed=$installed_path core=$core_path user=$user_path"
    return
  fi
  aletheon_ok "installed runtime provenance verified: sha256=$candidate_hash"
}

_service_snapshot() {
  local scope=$1 unit=$2 active pid restarts
  active=$(_service_property "$scope" "$unit" ActiveState)
  [[ "$active" == active ]] ||
    { aletheon_die "$scope service is not active: $unit ($active)"; return; }
  pid=$(_service_pid "$scope" "$unit") || return
  restarts=$(_service_property "$scope" "$unit" NRestarts)
  [[ "$restarts" =~ ^[0-9]+$ ]] ||
    { aletheon_die "$scope service has invalid restart count: $unit"; return; }
  printf '%s:%s\n' "$pid" "$restarts"
}

_service_has_fatal_log() {
  local scope=$1 unit=$2 pid=$3
  local args=(--no-pager --quiet -u "$unit" "_PID=$pid" -n 200)
  [[ "$scope" == user ]] && args=(--user "${args[@]}")
  journalctl "${args[@]}" 2>/dev/null |
    grep -Eqi 'references unknown tool|panic|Main process exited|Failed with result'
}

cmd_verify_runtime_stability() {
  [[ "$ALETHEON_STABILITY_SECONDS" =~ ^[0-9]+$ ]] ||
    { aletheon_die "ALETHEON_STABILITY_SECONDS must be a non-negative integer"; return; }
  local core_before user_before core_after user_after core_pid user_pid
  core_before=$(_service_snapshot system "$ALETHEON_CORE_UNIT") || return
  user_before=$(_service_snapshot user "$ALETHEON_USER_UNIT") || return
  sleep "$ALETHEON_STABILITY_SECONDS"
  core_after=$(_service_snapshot system "$ALETHEON_CORE_UNIT") || return
  user_after=$(_service_snapshot user "$ALETHEON_USER_UNIT") || return

  if [[ "$core_before" != "$core_after" || "$user_before" != "$user_after" ]]; then
    aletheon_die "service restart count increased during stability window: core=$core_before->$core_after user=$user_before->$user_after"
    return
  fi
  core_pid=${core_after%%:*}
  user_pid=${user_after%%:*}
  if _service_has_fatal_log system "$ALETHEON_CORE_UNIT" "$core_pid" ||
     _service_has_fatal_log user "$ALETHEON_USER_UNIT" "$user_pid"; then
    aletheon_die "fatal startup validation error detected for current runtime process"
    return
  fi
  aletheon_ok "runtime stability verified: interval=${ALETHEON_STABILITY_SECONDS}s"
}

cmd_verify_official_client() {
  local binary=${1:-$ALETHEON_INSTALLED_BINARY}
  [[ "$ALETHEON_SMOKE_TIMEOUT_SECONDS" =~ ^[1-9][0-9]*$ ]] ||
    { aletheon_die "ALETHEON_SMOKE_TIMEOUT_SECONDS must be a positive integer"; return; }
  local output status=0
  output=$(mktemp "${TMPDIR:-/tmp}/aletheon-deployment-smoke.XXXXXX") || return
  chmod 0600 "$output"
  if timeout "$ALETHEON_SMOKE_TIMEOUT_SECONDS" \
      "$binary" \
      --socket "$ALETHEON_USER_SOCKET" \
      -m "$ALETHEON_SMOKE_PROMPT" >"$output" 2>/dev/null; then
    :
  else
    status=$?
    rm -f -- "$output"
    aletheon_die "official client real-request smoke test failed (status=$status)"
    return
  fi
  if ! grep -q '[^[:space:]]' "$output"; then
    rm -f -- "$output"
    aletheon_die "official client real-request returned empty output"
    return
  fi
  rm -f -- "$output"
  aletheon_ok "official client real-request smoke test passed"
}

cmd_installed_runtime_gate() {
  cmd_verify_runtime_provenance || return
  cmd_verify_runtime_stability || return
  cmd_verify_official_client || return
  cmd_verify_runtime_provenance || return
  cmd_verify_runtime_stability || return
}

# User-mode variants: no system core service exists, and the installed binary
# lives under $HOME instead of /usr/bin. Provenance therefore compares only the
# release candidate, the user-installed binary, and the running user process.
_verify_user_provenance() {
  local bin=$1 candidate installed_path user_path
  local candidate_hash installed_hash user_hash
  candidate=$ALETHEON_RELEASE_BINARY
  [[ -x "$bin" ]] || { aletheon_die "installed user binary is unavailable: $bin"; return; }
  installed_path=$(readlink -f "$bin") ||
    { aletheon_die "cannot resolve installed user binary: $bin"; return; }
  user_path=$(_runtime_executable user "$ALETHEON_USER_UNIT") || return
  candidate_hash=$(_sha256 "$candidate") || return
  installed_hash=$(_sha256 "$bin") || return
  user_hash=$(_sha256 "$user_path") || return
  if [[ "$installed_hash" != "$candidate_hash" ]]; then
    aletheon_die "installed user binary hash differs from release candidate: candidate=$candidate_hash installed=$installed_hash"
    return
  fi
  if [[ "$user_hash" != "$candidate_hash" ]]; then
    aletheon_die "running user executable hash differs from release candidate: candidate=$candidate_hash user=$user_hash"
    return
  fi
  if [[ "$user_path" != "$installed_path" ]]; then
    aletheon_die "running user executable path differs from installed binary: installed=$installed_path user=$user_path"
    return
  fi
  aletheon_ok "installed user runtime provenance verified: sha256=$candidate_hash"
}

_verify_user_stability() {
  [[ "$ALETHEON_STABILITY_SECONDS" =~ ^[0-9]+$ ]] ||
    { aletheon_die "ALETHEON_STABILITY_SECONDS must be a non-negative integer"; return; }
  local before after pid
  before=$(_service_snapshot user "$ALETHEON_USER_UNIT") || return
  sleep "$ALETHEON_STABILITY_SECONDS"
  after=$(_service_snapshot user "$ALETHEON_USER_UNIT") || return
  if [[ "$before" != "$after" ]]; then
    aletheon_die "user service restart count increased during stability window: $before->$after"
    return
  fi
  pid=${after%%:*}
  if _service_has_fatal_log user "$ALETHEON_USER_UNIT" "$pid"; then
    aletheon_die "fatal startup validation error detected for current user runtime process"
    return
  fi
  aletheon_ok "user runtime stability verified: interval=${ALETHEON_STABILITY_SECONDS}s"
}

cmd_installed_runtime_gate_user() {
  local bin=${1:-${ALETHEON_USER_BIN_DIR:-$HOME/.local/bin}/aletheon}
  _verify_user_provenance "$bin" || return
  _verify_user_stability || return
  cmd_verify_official_client "$bin" || return
  _verify_user_provenance "$bin" || return
  _verify_user_stability || return
}
