#!/usr/bin/env bash
set -euo pipefail

root=${ALETHEON_DATA_ROOT:-/var/lib/aletheon}
cache_root=${ALETHEON_CACHE_ROOT:-/var/cache/aletheon}
now=$(date +%s)
removed=0

[[ -d "$root" && ! -L "$root" ]] || { echo "invalid data root" >&2; exit 1; }

# Cleanup is opt-in per entry: producers write a regular `.cleanup-after` file
# containing an epoch after they have durably acknowledged/verified the entry.
# Active/pinned/evidence entries are fail-closed and never removed.
for class in worktrees sessions artifacts; do
  directory=$root/$class
  [[ -d "$directory" && ! -L "$directory" ]] || continue
  while IFS= read -r -d '' marker; do
    entry=$(dirname -- "$marker")
    [[ ! -e "$entry/.active" && ! -e "$entry/.pinned" && ! -e "$entry/.legal-hold" ]] || continue
    [[ -f "$marker" && ! -L "$marker" ]] || continue
    read -r expires <"$marker" || continue
    [[ "$expires" =~ ^[0-9]+$ && "$expires" -le "$now" ]] || continue
    canonical=$(realpath -e -- "$entry")
    case "$canonical" in "$directory"/*) ;; *) echo "cleanup entry escaped managed root" >&2; exit 1 ;; esac
    find "$canonical" -xdev -type l -print -quit | grep -q . && { echo "refusing symlinked cleanup entry" >&2; exit 1; }
    rm -rf --one-file-system -- "$canonical"
    removed=$((removed + 1))
  done < <(find "$directory" -mindepth 2 -maxdepth 2 -name .cleanup-after -type f -print0)
done
directory=$cache_root
if [[ -d "$directory" && ! -L "$directory" ]]; then
  while IFS= read -r -d '' marker; do
    entry=$(dirname -- "$marker")
    [[ ! -e "$entry/.active" && ! -e "$entry/.pinned" && ! -e "$entry/.legal-hold" ]] || continue
    read -r expires <"$marker" || continue
    [[ "$expires" =~ ^[0-9]+$ && "$expires" -le "$now" ]] || continue
    canonical=$(realpath -e -- "$entry")
    case "$canonical" in "$directory"/*) ;; *) echo "cleanup cache entry escaped managed root" >&2; exit 1 ;; esac
    find "$canonical" -xdev -type l -print -quit | grep -q . && { echo "refusing symlinked cleanup cache entry" >&2; exit 1; }
    rm -rf --one-file-system -- "$canonical"
    removed=$((removed + 1))
  done < <(find "$directory" -mindepth 2 -maxdepth 3 -name .cleanup-after -type f -print0)
fi
echo "managed cleanup complete: removed=$removed" >&2
