#!/usr/bin/env bash
set -euo pipefail

failures=0
fail() { echo "FAIL: $*" >&2; failures=$((failures + 1)); }

validate_dir() {
  local root=${1:-/etc/aletheon/credentials} file mode owner parent pmode
  [[ -d "$root" && ! -L "$root" ]] || { fail "credential root is absent, not a directory, or a symlink: $root"; return; }
  parent=$root
  while [[ "$parent" != / ]]; do
    [[ ! -L "$parent" ]] || fail "credential ancestor is a symlink: $parent"
    pmode=$(stat -Lc '%a' "$parent") || { fail "cannot stat credential ancestor: $parent"; break; }
    (( (8#$pmode & 0002) == 0 )) || fail "credential ancestor is world-writable: $parent"
    parent=$(dirname -- "$parent")
  done
  for file in "$root"/*; do
    [[ -e "$file" ]] || continue
    [[ -f "$file" && ! -L "$file" ]] || { fail "credential is not a regular non-symlink file: $file"; continue; }
    mode=$(stat -Lc '%a' "$file")
    [[ "$mode" == 600 ]] || fail "credential mode must be 0600: $file ($mode)"
    owner=$(stat -Lc '%U:%G' "$file")
    [[ "$owner" == aletheon:aletheon || "$owner" == root:aletheon ]] || fail "unexpected credential owner: $file ($owner)"
  done
  if [[ -e "$root/google-vault.key" ]]; then
    [[ $(stat -Lc '%s' "$root/google-vault.key") -eq 32 ]] || fail "Google vault key must contain exactly 32 bytes"
  fi
  if command -v git >/dev/null && git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    while IFS= read -r file; do
      case "$(realpath -m -- "$file")" in "$root"/*) fail "credential is tracked by Git: $file" ;; esac
    done < <(git ls-files)
  fi
}

scan_canary() {
  local canary_file=$1 root=${2:-/etc/aletheon/credentials} hit=0
  [[ -f "$canary_file" && ! -L "$canary_file" ]] || { fail "canary file must be a regular non-symlink file"; return; }
  [[ $(stat -Lc '%a' "$canary_file") == 600 ]] || fail "canary file mode must be 0600"
  [[ -s "$canary_file" ]] || { fail "canary file is empty"; return; }

  # grep receives the pattern through a protected file, not argv. It reports
  # only category names and never the matching line or value.
  if git rev-parse --is-inside-work-tree >/dev/null 2>&1 && \
     git grep -qFf "$canary_file" -- ':!*.secret-canary' 2>/dev/null; then
    fail "canary found in Git-tracked content"; hit=1
  fi
  if [[ -r /proc/$$/cmdline ]] && grep -aqFf "$canary_file" /proc/$$/cmdline; then
    fail "canary found in process argv"; hit=1
  fi
  if [[ -r /proc/$$/environ ]] && grep -aqFf "$canary_file" /proc/$$/environ; then
    fail "canary found in process environment"; hit=1
  fi
  for scope in /var/lib/aletheon/audit /var/lib/aletheon/artifacts /var/lib/aletheon/sessions /var/cache/aletheon; do
    [[ -d "$scope" ]] || continue
    if grep -rIlFf "$canary_file" "$scope" --exclude='*.secret-canary' 2>/dev/null | grep -q .; then
      fail "canary found in runtime data category: $scope"; hit=1
    fi
  done
  if command -v journalctl >/dev/null && journalctl -u aletheon.service --no-pager -o cat 2>/dev/null | grep -qFf "$canary_file"; then
    fail "canary found in service journal"; hit=1
  fi
  ((hit == 0)) && echo "secret canary absent from audited plaintext scopes" >&2
  : "$root" # approved encrypted credential root is intentionally not scanned
}

case ${1:-} in
  --validate) validate_dir "${2:-/etc/aletheon/credentials}" ;;
  --canary-file)
    [[ $# -ge 2 && $# -le 3 ]] || { echo "usage: $0 --canary-file FILE [credential-root]" >&2; exit 64; }
    validate_dir "${3:-/etc/aletheon/credentials}"
    scan_canary "$2" "${3:-/etc/aletheon/credentials}"
    ;;
  *) echo "usage: $0 --validate [credential-root] | --canary-file FILE [credential-root]" >&2; exit 64 ;;
esac

((failures == 0)) || exit 1
echo "secret audit passed" >&2
