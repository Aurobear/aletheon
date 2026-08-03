# aurb Aletheon Extension Implementation Plan

> **For agentic workers:** Use `flow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make aurb build, validate, install, enable, upgrade, disable, and roll back a real multi-asset Aletheon extension package.

**Architecture:** aurb gains a deterministic package builder that reads an explicit non-secret asset selection, converts compatible roles/hooks/MCP declarations into Aletheon's existing package contract, and invokes only public `aletheon extension` commands. Aletheon remains the package, permission, secret, runtime, and rollback authority; aurb never writes Aletheon state directories.

**Tech Stack:** Bash entry point, Python 3 standard library plus existing PyYAML, deterministic tar/gzip, Aletheon extension v1 manifests, pytest, official `/usr/bin/aletheon` CLI/socket.

**Prerequisite:** Complete `docs/plans/2026-08-03-extension-runtime-activation.md` through Task 9 before installed-runtime acceptance.

**Approved design:** `docs/plans/2026-08-03-governed-commands-extension-runtime-design.md:284-336`

**Target repository:** `/home/aurobear/Workspace/agent/aurb`

---

### Task 1: Declare the aurb package asset selection

**Files:**
- Create: `/home/aurobear/Workspace/agent/aurb/config/aletheon-extension.yaml`
- Modify: `/home/aurobear/Workspace/agent/aurb/config/config.yaml.example`
- Create: `/home/aurobear/Workspace/agent/aurb/tests/aletheon/test_extension_config.py`

- [ ] **Step 1: Write the failing configuration test**

```python
from pathlib import Path
import yaml

ROOT = Path(__file__).resolve().parents[2]


def test_extension_selection_is_non_secret_and_paths_exist():
    config = yaml.safe_load((ROOT / "config/aletheon-extension.yaml").read_text())
    assert config["schema_version"] == 1
    assert config["package"]["id"] == "aurb.core"
    serialized = yaml.safe_dump(config)
    for forbidden in ["api_key", "token:", "password", "secret_value"]:
        assert forbidden not in serialized.lower()
    for section in ["skills", "hooks", "connectors", "profiles"]:
        for entry in config["assets"].get(section, []):
            assert (ROOT / entry["source"]).is_file(), entry["source"]
```

- [ ] **Step 2: Run and confirm missing-file failure**

```bash
cd /home/aurobear/Workspace/agent/aurb
.venv/bin/python -m pytest tests/aletheon/test_extension_config.py -q
```

Expected: fail because `config/aletheon-extension.yaml` does not exist.

- [ ] **Step 3: Add the explicit checked-in selection**

```yaml
schema_version: 1
package:
  id: aurb.core
  description: Governed aurb assets for Aletheon
  min_aletheon: 0.1.0

permissions:
  network: true
  executables: true
  filesystem: []

assets:
  skills:
    - id: skill.code-analyzer
      source: src/skills/analysis/code-analyzer/SKILL.md
    - id: skill.requirements
      source: src/skills/analysis/requirements/SKILL.md
    - id: skill.system-analyzer
      source: src/skills/analysis/system-analyzer/SKILL.md
    - id: skill.knowledge
      source: src/skills/analysis/knowledge/SKILL.md
  hooks:
    - id: hook.health-evidence
      source: src/aletheon/hooks/health-evidence.toml
  connectors:
    - id: connector.gbrain
      source: src/aletheon/connectors/gbrain.json
  profiles:
    - id: agent.reviewer
      source: src/aletheon/agents/reviewer.md
```

Add an `aletheon_extension` section to `config.yaml.example` containing only
enablement, output directory, and secret-reference names:

```yaml
aletheon_extension:
  enabled: true
  manifest: config/aletheon-extension.yaml
  output_dir: dist/aletheon
  connector_secrets:
    gbrain_authorization: GBRAIN_READ_TOKEN
```

- [ ] **Step 4: Run and commit**

```bash
.venv/bin/python -m pytest tests/aletheon/test_extension_config.py -q
```

Expected: pass after Task 2 supplies the three referenced Aletheon-specific source files; until then the test remains intentionally red and is committed with Task 2 rather than alone.

### Task 2: Add Aletheon-native Hook, Connector, and Profile assets

**Files:**
- Create: `/home/aurobear/Workspace/agent/aurb/src/aletheon/hooks/health-evidence.toml`
- Create: `/home/aurobear/Workspace/agent/aurb/src/aletheon/payload/hooks/health-evidence.sh`
- Create: `/home/aurobear/Workspace/agent/aurb/src/aletheon/connectors/gbrain.json`
- Create: `/home/aurobear/Workspace/agent/aurb/src/aletheon/agents/reviewer.md`
- Create: `/home/aurobear/Workspace/agent/aurb/tests/aletheon/test_native_assets.py`
- Test: `/home/aurobear/Workspace/agent/aurb/tests/aletheon/test_extension_config.py`

- [ ] **Step 1: Write native asset contract tests**

```python
import json
from pathlib import Path
import yaml

ROOT = Path(__file__).resolve().parents[2]


def test_hook_script_is_package_relative_and_non_mutating():
    hook = yaml.safe_load((ROOT / "src/aletheon/hooks/health-evidence.toml").read_text())
    assert hook["hook"]["point"] == "PostTurn"
    assert hook["hook"]["script"] == "payload/hooks/health-evidence.sh"
    script = (ROOT / "src/aletheon/payload/hooks/health-evidence.sh").read_text()
    assert "aletheon --message" not in script
    assert "extension enable" not in script


def test_connector_uses_secret_reference_not_secret_value():
    connector = json.loads((ROOT / "src/aletheon/connectors/gbrain.json").read_text())
    assert connector["schema_version"] == 1
    assert connector["transport"]["kind"] == "streamable_http"
    assert connector["bearer_token_env"] == "GBRAIN_READ_TOKEN"


def test_profile_uses_aletheon_tool_names():
    profile = (ROOT / "src/aletheon/agents/reviewer.md").read_text()
    assert "tools: [file_read, grep, glob, git_diff, aurb_gbrain__search, aurb_gbrain__get_page]" in profile
    assert "tools: Read" not in profile
```

- [ ] **Step 2: Run and confirm missing-file failure**

```bash
.venv/bin/python -m pytest tests/aletheon/test_native_assets.py -q
```

Expected: fail because the native assets do not exist.

- [ ] **Step 3: Add the assets**

`health-evidence.toml`:

```toml
[hook]
name = "aurb-health-evidence"
point = "PostTurn"
priority = 90
script = "payload/hooks/health-evidence.sh"
timeout_ms = 2000
```

`health-evidence.sh` reads the typed event JSON from stdin, validates it with
Python's JSON parser, invokes only `/usr/bin/aletheon doctor --json`, and emits a
single JSON result to stdout. It does not submit a model turn, mutate extension
state, write memory, or access credentials.

`gbrain.json`:

```json
{
  "schema_version": 1,
  "id": "aurb.gbrain",
  "transport": {
    "kind": "streamable_http",
    "url": "http://127.0.0.1:3131/mcp"
  },
  "bearer_token_env": "GBRAIN_READ_TOKEN",
  "request_timeout_ms": 2000,
  "allowed_tools": ["search", "get_page"],
  "allowed_resources": []
}
```

`reviewer.md` reuses the existing reviewer body but has Aletheon-native
frontmatter. The two Connector tools are explicit Profile dependencies so the
real installed acceptance exercises GBrain through the authorized candidate
snapshot:

```yaml
---
name: aurb-reviewer
description: Independently review changed files and validation evidence
tools: [file_read, grep, glob, git_diff, aurb_gbrain__search, aurb_gbrain__get_page]
max_iterations: 20
---
```

Do not reuse Claude's `model: haiku` or its `Read/Grep/Glob/Bash` grants.

- [ ] **Step 4: Run both test modules and commit**

```bash
.venv/bin/python -m pytest tests/aletheon/test_extension_config.py tests/aletheon/test_native_assets.py -q
```

Expected: pass.

```bash
git add config/aletheon-extension.yaml config/config.yaml.example src/aletheon tests/aletheon/test_extension_config.py tests/aletheon/test_native_assets.py
git commit -F - <<'MSG'
feat(aletheon): declare native extension assets

Claude and Codex runtime files are not automatically valid Aletheon package
assets, and connector credentials must remain host-owned.

- select the initial governed aurb asset set
- add an Aletheon-native Hook and Agent Profile
- declare GBrain through a secret reference
MSG
```

### Task 3: Build deterministic extension archives

**Files:**
- Create: `/home/aurobear/Workspace/agent/aurb/scripts/lib/aletheon_extension.py`
- Create: `/home/aurobear/Workspace/agent/aurb/tests/aletheon/test_package_builder.py`
- Modify: `/home/aurobear/Workspace/agent/aurb/.gitignore`

- [ ] **Step 1: Write deterministic archive tests**

```python
import hashlib
import tarfile
from pathlib import Path
from scripts.lib.aletheon_extension import build_package


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def test_build_is_reproducible_and_complete(tmp_path):
    first = build_package(manifest_path(), tmp_path / "one", version="1.2.3", epoch=1_786_000_000)
    second = build_package(manifest_path(), tmp_path / "two", version="1.2.3", epoch=1_786_000_000)
    assert sha(first) == sha(second)
    with tarfile.open(first, "r:gz") as archive:
        names = sorted(archive.getnames())
    assert "extension.toml" in names
    assert "checksums.sha256" in names
    assert "assets/skills/code-analyzer/SKILL.md" in names
    assert "assets/hooks/health-evidence.toml" in names
    assert "assets/connectors/gbrain.json" in names
    assert "assets/agents/reviewer.md" in names
    assert "payload/hooks/health-evidence.sh" in names
```

- [ ] **Step 2: Run and confirm import failure**

```bash
.venv/bin/python -m pytest tests/aletheon/test_package_builder.py -q
```

Expected: import failure because `aletheon_extension.py` is absent.

- [ ] **Step 3: Implement the builder**

Expose:

```python
def build_package(
    selection_path: Path,
    output_dir: Path,
    *,
    version: str,
    epoch: int,
) -> Path:
    """Build and return a deterministic aurb Aletheon .tar.gz archive."""
```

The builder must:

1. Parse and validate `schema_version: 1`.
2. Reject absolute paths, `..`, missing files, duplicate asset IDs, and secret-like
   keys/values.
3. Map selected files to exact package paths:
   - Skills: `assets/skills/<name>/SKILL.md`, including sibling references used
     by each selected Skill.
   - Hooks: `assets/hooks/<name>.toml` plus declared `payload/` scripts.
   - Connectors: `assets/connectors/<name>.json`.
   - Profiles: `assets/agents/<name>.md`.
4. Generate `extension.toml` using package ID `aurb.core`, the supplied version,
   compatibility, exact asset refs, and requested permissions.
5. Generate sorted lowercase SHA-256 lines for every file except
   `checksums.sha256`.
6. Write tar entries sorted by path with uid/gid 0, empty user/group names,
   mode `0644` for data and `0755` only for payload executables, and `mtime=epoch`.
7. Write gzip with `mtime=epoch` and no original filename.
8. Name the result `aurb-core-<version>.tar.gz`.

Add `/dist/aletheon/` to `.gitignore`.

- [ ] **Step 4: Run and commit**

```bash
.venv/bin/python -m pytest tests/aletheon/test_package_builder.py -q
```

Expected: pass, including identical hashes from two builds.

```bash
git add scripts/lib/aletheon_extension.py tests/aletheon/test_package_builder.py .gitignore
git commit -F - <<'MSG'
feat(aletheon): build deterministic extension archives

Aletheon installation requires a checksummed package rather than direct copies
of aurb runtime files.

- validate selected assets and generate extension.toml
- produce complete sorted checksum coverage
- normalize tar and gzip metadata for reproducible output
MSG
```

### Task 4: Add aurb package/install lifecycle commands

**Files:**
- Create: `/home/aurobear/Workspace/agent/aurb/scripts/lib/aletheon.sh`
- Modify: `/home/aurobear/Workspace/agent/aurb/scripts/aurb.sh`
- Modify: `/home/aurobear/Workspace/agent/aurb/scripts/lib/deploy.sh`
- Create: `/home/aurobear/Workspace/agent/aurb/tests/aletheon/test_cli.sh`

- [ ] **Step 1: Write command routing tests with a fake Aletheon CLI**

```bash
#!/usr/bin/env bash
set -euo pipefail
root=$(cd -- "$(dirname -- "$0")/../.." && pwd -P)
tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
cat >"$tmp/aletheon" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"$ALETHEON_CAPTURE"
SH
chmod +x "$tmp/aletheon"
export PATH="$tmp:$PATH" ALETHEON_CAPTURE="$tmp/calls"

bash "$root/scripts/aurb.sh" aletheon package --version 1.2.3 --output "$tmp/dist"
bash "$root/scripts/aurb.sh" aletheon validate "$tmp/dist/aurb-core-1.2.3.tar.gz"
bash "$root/scripts/aurb.sh" aletheon install "$tmp/dist/aurb-core-1.2.3.tar.gz" --trust-workspace

grep -qx 'extension validate .*aurb-core-1.2.3.tar.gz' "$tmp/calls"
grep -qx 'extension install .*aurb-core-1.2.3.tar.gz --trust-workspace' "$tmp/calls"
grep -qx 'extension enable aurb.core --approve-permissions' "$tmp/calls"
```

- [ ] **Step 2: Run and confirm unknown-command failure**

```bash
bash tests/aletheon/test_cli.sh
```

Expected: exit 2 with `Unknown command: aletheon`.

- [ ] **Step 3: Implement the lifecycle wrapper**

Add dispatch:

```bash
aletheon) shift; cmd_aletheon "$@" ;;
```

Supported commands:

```text
aurb.sh aletheon package [--version V] [--output DIR]
aurb.sh aletheon validate ARCHIVE
aurb.sh aletheon install ARCHIVE [--trust-workspace]
aurb.sh aletheon enable [--approve-permissions]
aurb.sh aletheon disable
aurb.sh aletheon status
aurb.sh aletheon upgrade ARCHIVE [--trust-workspace] [--approve-permissions]
aurb.sh aletheon rollback
aurb.sh aletheon uninstall
aurb.sh deploy aletheon
```

`install` runs validate, install, enable, and doctor in that order and stops on
first failure. It forwards explicit approval flags but never adds them silently.
`uninstall` maps to `extension remove`, not purge. `deploy aletheon` builds from
the checked-in selection then installs; `deploy all` includes Aletheon only when
`aletheon_extension.enabled` is true.

- [ ] **Step 4: Run and commit**

```bash
bash -n scripts/aurb.sh scripts/lib/aletheon.sh scripts/lib/deploy.sh
bash tests/aletheon/test_cli.sh
```

Expected: pass.

```bash
git add scripts/aurb.sh scripts/lib/aletheon.sh scripts/lib/deploy.sh tests/aletheon/test_cli.sh
git commit -F - <<'MSG'
feat(aletheon): manage aurb extension lifecycle

Users need one supported aurb workflow for package construction and governed
Aletheon installation.

- add package, validate, install, upgrade, rollback, and disable commands
- require explicit workspace trust and permission approvals
- integrate Aletheon with configured aurb deployment targets
MSG
```

### Task 5: Extend validation and completion

**Files:**
- Modify: `/home/aurobear/Workspace/agent/aurb/scripts/lib/validate.sh`
- Modify: `/home/aurobear/Workspace/agent/aurb/scripts/lib/test.sh`
- Create: `/home/aurobear/Workspace/agent/aurb/scripts/completions/aurb.bash`
- Create: `/home/aurobear/Workspace/agent/aurb/scripts/completions/aurb.zsh`
- Modify: `/home/aurobear/Workspace/agent/aurb/scripts/lib/deploy.sh:435-465,606-626`
- Create: `/home/aurobear/Workspace/agent/aurb/tests/aletheon/test_validation.py`

- [ ] **Step 1: Add validation failures for unsafe assets**

```python
import pytest
from scripts.lib.aletheon_extension import load_selection


def test_rejects_plaintext_connector_secret(tmp_path):
    selection = fixture_selection(tmp_path)
    connector = tmp_path / "connector.json"
    connector.write_text('{"bearer_token_env":"Bearer abc"}')
    selection["assets"]["connectors"][0]["source"] = str(connector)
    with pytest.raises(ValueError, match="secret reference"):
        load_selection(selection)
```

- [ ] **Step 2: Implement validation integration**

`cmd_validate` calls the Python builder in `--check` mode and reports invalid
paths, duplicate IDs, unknown asset types, secret values, non-Aletheon Profile
tool names, and missing payload scripts. It does not require a running daemon.

Completion adds the exact `aletheon` subcommands and known value choices. It
completes archive arguments as files and `--output` as a directory.
`cmd_install_runtime` installs both definitions with mode `0644` under
`$XDG_DATA_HOME/bash-completion/completions/aurb` and
`$XDG_DATA_HOME/zsh/site-functions/_aurb`; a new shell activates them.

- [ ] **Step 3: Run and commit**

```bash
.venv/bin/python -m pytest tests/aletheon -q
bash scripts/aurb.sh validate
bash scripts/aurb.sh test --skip-daemon
```

Expected: pass.

```bash
git add scripts/lib/validate.sh scripts/lib/test.sh scripts/lib/deploy.sh scripts/completions/aurb.bash scripts/completions/aurb.zsh tests/aletheon/test_validation.py
git commit -F - <<'MSG'
test(aletheon): validate aurb extension inputs

Package generation must reject unsafe or incompatible custom assets before an
operator reaches the Aletheon approval boundary.

- integrate extension checks with aurb validation
- cover plaintext secrets and invalid asset paths
- complete the new lifecycle command tree
MSG
```

### Task 6: Run real installed-runtime acceptance

**Files:**
- Create: `/home/aurobear/Workspace/agent/aurb/tests/aletheon/installed_acceptance.sh`
- Modify: `/home/aurobear/Workspace/agent/aurb/docs/deployment-verification.md`

- [ ] **Step 1: Add the authoritative acceptance script**

The script must refuse non-system clients:

```bash
test "$(readlink -f "$(command -v aletheon)")" = /usr/bin/aletheon
repo_sha=$(sha256sum /home/aurobear/Workspace/aletheon/target/release/aletheon | awk '{print $1}')
installed_sha=$(sha256sum /usr/bin/aletheon | awk '{print $1}')
test "$repo_sha" = "$installed_sha"
```

It then:

1. Builds the real aurb package twice and compares SHA-256.
2. Runs `/usr/bin/aletheon extension validate`.
3. Installs and enables `aurb.core` through the official socket.
4. Reads `/skills` and proves `aurb:code-analyzer` is active.
5. Starts a real unchanged TUI session, executes a task requiring the packaged
   Skill, then a follow-up turn.
6. Observes a real PostTurn Hook terminal receipt.
7. Calls packaged GBrain MCP `search` and checks the authoritative terminal
   result; unavailable configured GBrain is a failure, not a skip.
8. Selects the packaged reviewer Profile and verifies typed host state reports
   it as the selected runtime.
9. Disables `aurb.core` and proves Skill, Hook, connector tools, and Profile are
   all absent.
10. Re-enables, upgrades a changed version, injects a failing upgrade, proves the
    previous digest remains active, then rolls back.
11. Restarts the user daemon and proves the snapshot digest is unchanged.
12. Samples `NRestarts` before and after the stability interval.

- [ ] **Step 2: Run prerequisite deterministic checks**

```bash
cd /home/aurobear/Workspace/agent/aurb
.venv/bin/python -m pytest tests/aletheon -q
bash tests/aletheon/test_cli.sh
bash scripts/aurb.sh validate
```

Expected: pass.

- [ ] **Step 3: Deploy Aletheon and run installed acceptance**

From `/home/aurobear/Workspace/aletheon`:

```bash
sudo bash scripts/aletheon.sh deploy
```

Expected: deployment verification passes, official real-request smoke passes,
and release/installed/running digests match.

Then:

```bash
cd /home/aurobear/Workspace/agent/aurb
bash tests/aletheon/installed_acceptance.sh
```

Expected: `aurb Aletheon extension acceptance: pass` with receipt paths and
snapshot digests printed.

- [ ] **Step 4: Commit acceptance and documentation**

```bash
git add tests/aletheon/installed_acceptance.sh docs/deployment-verification.md
git commit -F - <<'MSG'
test(aletheon): accept governed aurb installation

A package builder is complete only when the system-installed Aletheon runtime
can activate and execute every selected aurb asset kind.

- verify real Skill, Hook, MCP, and Profile behavior
- exercise disable, upgrade failure, rollback, and restart recovery
- require official socket, installed provenance, and stable daemons
MSG
```
