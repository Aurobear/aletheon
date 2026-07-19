#!/usr/bin/env bash
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)
python3 "$root/tests/coding/replay_test.py"
python3 - <<'PY' "$root"
import pathlib,sys,tomllib
root=pathlib.Path(sys.argv[1]); required={"id","fixture","prompt","timeout_secs","acceptance_commands","forbidden_paths"}
for path in sorted((root/"tests/coding/tasks").glob("*.toml")):
    task=tomllib.loads(path.read_text()); assert set(task)==required, (path,set(task)); assert task["acceptance_commands"]
assert len(list((root/"tests/coding/tasks").glob("*.toml")))==3
PY
for fixture in rust_bugfix rust_multifile rust_diagnosis; do
  manifest="$root/tests/coding/fixtures/$fixture/Cargo.toml"
  grep -Fxq '[workspace]' "$manifest"
done
# Harness invariants: real binary, process-group timeout cleanup, independent
# acceptance, operation correlation, bounded output, and integrity sealing.
grep -Fq 'ALETHEON_BIN' "$root/tests/coding/harness/run.py"
grep -Fq 'start_new_session=True' "$root/tests/coding/harness/run.py"
grep -Fq 'os.killpg' "$root/tests/coding/harness/run.py"
grep -Fq 'acceptance_commands' "$root/tests/coding/harness/run.py"
grep -Fq 'operation_id' "$root/tests/coding/harness/run.py"
grep -Fq 'MAX_CAPTURE' "$root/tests/coding/harness/run.py"
grep -Fq 'integrity_sha256' "$root/tests/coding/harness/run.py"
echo 'coding harness static verification: pass'
