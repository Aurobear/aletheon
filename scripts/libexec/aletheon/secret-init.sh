#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'EOF'
Usage:
  aletheon-secret-init.sh init [credential-directory]
  aletheon-secret-init.sh rotate NAME [credential-directory] < new-secret

The rotate command reads the replacement from standard input. Secret values are
never accepted as command-line arguments or printed.
EOF
  exit 64
}

[[ ${EUID:-$(id -u)} -eq 0 ]] || { echo "run as root" >&2; exit 1; }
command=${1:-init}
name=
case "$command" in
  init) root=${2:-/etc/aletheon/credentials}; [[ $# -le 2 ]] || usage ;;
  rotate) name=${2:-}; root=${3:-/etc/aletheon/credentials}; [[ -n "$name" && $# -le 3 ]] || usage ;;
  *) usage ;;
esac

case "$root" in /*) ;; *) echo "credential directory must be absolute" >&2; exit 1 ;; esac
getent group aletheon >/dev/null || { echo "aletheon group does not exist" >&2; exit 1; }
id -u aletheon >/dev/null 2>&1 || { echo "aletheon user does not exist" >&2; exit 1; }
umask 077
install -d -o root -g aletheon -m 0750 "$root"

atomic_install() {
  local target=$1 owner=$2 group=$3 tmp
  [[ ! -L "$target" ]] || { echo "refusing symlink: $target" >&2; return 1; }
  tmp=$(mktemp --tmpdir="$root" .credential.XXXXXX)
  trap 'rm -f -- "${tmp:-}"' RETURN
  cat >"$tmp"
  [[ -s "$tmp" ]] || { echo "refusing empty credential" >&2; return 1; }
  chown "$owner:$group" "$tmp"
  chmod 0600 "$tmp"
  sync -f "$tmp" 2>/dev/null || true
  mv -fT -- "$tmp" "$target"
  sync -d "$root" 2>/dev/null || true
  trap - RETURN
}

if [[ "$command" == init ]]; then
  if [[ ! -e "$root/google-vault.key" ]]; then
    # dd writes kernel CSPRNG bytes directly to the temporary file; no secret is
    # expanded by the shell, printed, or exposed in argv.
    tmp=$(mktemp --tmpdir="$root" .vault-key.XXXXXX)
    trap 'rm -f -- "${tmp:-}"' EXIT
    dd if=/dev/urandom of="$tmp" bs=32 count=1 status=none
    chown root:aletheon "$tmp"
    chmod 0600 "$tmp"
    mv -T -- "$tmp" "$root/google-vault.key"
    sync -d "$root" 2>/dev/null || true
    trap - EXIT
  fi
  for file in provider.env telegram.env gbrain.env restic-password restic-repository; do
    [[ -e "$root/$file" ]] || install -o aletheon -g aletheon -m 0600 /dev/null "$root/$file"
  done
  echo "credential files initialized at $root" >&2
  exit 0
fi

case "$name" in
  provider.env|telegram.env|google-vault.key|gbrain.env|restic-password|restic-repository) ;;
  *) echo "unsupported credential name" >&2; exit 64 ;;
esac
if [[ "$name" == google-vault.key ]]; then
  atomic_install "$root/$name" root aletheon
else
  atomic_install "$root/$name" aletheon aletheon
fi
echo "credential replaced atomically: $name" >&2
