#!/usr/bin/env bash
set -euo pipefail

if (( $# == 0 )); then
  echo "rustc-sccache-fallback: missing rustc executable" >&2
  exit 2
fi

rustc=$1
shift
stdout_file=$(mktemp "${TMPDIR:-/tmp}/aletheon-sccache-stdout.XXXXXX")
stderr_file=$(mktemp "${TMPDIR:-/tmp}/aletheon-sccache-stderr.XXXXXX")
trap 'rm -f -- "$stdout_file" "$stderr_file"' EXIT

if sccache "$rustc" "$@" >"$stdout_file" 2>"$stderr_file"; then
  cat -- "$stdout_file"
  cat >&2 -- "$stderr_file"
  exit 0
else
  status=$?
fi

cat >&2 -- "$stderr_file"
if [[ ${SCCACHE_IGNORE_SERVER_IO_ERROR:-1} != 0 ]] \
  && grep -Eq \
    'Failed to send data to or receive data from server|Failed to read response header|Connection reset by peer|Connection refused|Broken pipe' \
    "$stderr_file"; then
  echo "Aletheon Cargo: sccache server I/O failure; retrying rustc locally" >&2
  rm -f -- "$stdout_file" "$stderr_file"
  trap - EXIT
  exec "$rustc" "$@"
fi

cat -- "$stdout_file"
exit "$status"
