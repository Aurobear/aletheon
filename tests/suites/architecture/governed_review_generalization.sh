#!/usr/bin/env bash
set -euo pipefail
repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../.." && pwd -P)
cd "$repo_root"

paths=(
  crates/fabric/src/types/governed_review.rs
  crates/executive/src/application/governed_review
  crates/executive/src/composition/config/governed_review.rs
  crates/executive/src/host/daemon/handler/rpc/rpc_review.rs
  scripts/libexec/aletheon/governed-review-smoke.py
)
for path in "${paths[@]}"; do
  [[ -e "$path" ]] || { echo "missing governed review production path: $path" >&2; exit 1; }
done

if grep -RniE -- 'aurb|gbrain|aurobear|Workspace/(agent|work)|/home/[^/]+/' "${paths[@]}"; then
  echo 'governed review production surface contains consumer-specific coupling' >&2
  exit 1
fi
if grep -RniE -- 'Command::new|std::process|tokio::process|bash_exec|exec_command' \
  crates/executive/src/application/governed_review \
  crates/executive/src/host/daemon/handler/rpc/rpc_review.rs; then
  echo 'governed review service must not invoke external commands or consumer CLIs' >&2
  exit 1
fi
grep -q 'tools: vec!\[\]' crates/executive/src/application/governed_review/service.rs
grep -q 'connection.principal_id' crates/executive/src/host/daemon/handler/rpc/rpc_review.rs
echo 'governed review generalization: pass'
