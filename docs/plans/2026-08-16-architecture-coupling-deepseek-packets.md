# 耦合收敛：DeepSeek 任务包

状态：P0–P2 已获准执行；Finding F 仍是 `NEEDS EVIDENCE`，不属于任务包。

上位计划：[`2026-08-16-architecture-coupling-closeout.md`](./2026-08-16-architecture-coupling-closeout.md)

诊断：[`docs/arch/evidence/2026-08-16-architecture-coupling-diagnosis.md`](../arch/evidence/2026-08-16-architecture-coupling-diagnosis.md)

一次只领一个包。做完验证、写出回执、停下等人审。禁止 `git add -A`。禁止开始下一个包。Finding F 不是任务包。

```text
CC-P0.1 -> CC-P0.2 -> CC-P0.3 -> CC-P1.1 -> CC-P1.2 -> CC-P1.3 -> CC-P2.1 -> CC-P2.2 -> CC-P2.3
```

---

## 0. 交给 DeepSeek 的第一句话

复制下面整段，不要附加诊断全文，也不要说“把 P0–P2 一起做完”。

```text
只领取 docs/plans/2026-08-16-architecture-coupling-deepseek-packets.md 的 CC-P0.1。
先跑包内前置命令并输出回执，再改允许的文件。
完成后跑验证命令，写完成报告，停下等人审。
禁止 git add -A。禁止开始 CC-P0.2 或任何代码包。
诊断已批准 P0–P2；本包只改架构总览文档。
```

后续每个包把 `CC-P0.1` 换成下一个编号，其余句子不变。

---

## 1. DeepSeek 可以 / 不可以

可以：

- 读当前 checkout、诊断、本文件、closeout、相关 TSV/`Cargo.toml`
- 在当前分支做**一个**包的允许文件
- 发现 locator 与源码不符时停下来，只改本包文档/账本 locator，不开新需求

不可以：

- 一次做两个包，或把 P0 和 P1 捆在一起
- 拆新 crate，或把 `TurnPipeline` / `AgentControl` 整包搬出 `aletheon`
- 新建 `SessionTurnEngine` 类型（权威名是已有 `TurnEngine`）
- 把 Finding F（背压普查、重启语义、安装态）做成实现
- 为让 gate 变绿加 compatibility seam
- 提交工作区里与本包无关的迁移文件

事实源：当前生产路径 > 诊断 AGREE 项 > closeout > 本任务包 > 旧文档。

---

## 2. 每个包开始前的回执

改文件前必须输出（缺一项就停）：

```text
Slice:
Baseline commit:
Prerequisites:
Allowed files exist: yes/no + paths
Forbidden files will not be touched: yes
Locator re-check: command + result
Unknowns:
```

---

## 3. 任务包

### CC-P0.1 重写架构总览

前置：

```bash
test ! -d crates/executive
test ! -d crates/fabric
test -f docs/design/architecture-overview.md
test -f crates/aletheon/src/wiring/application/turn_engine.rs
```

允许改：`docs/design/architecture-overview.md`

禁止改：任何 `.rs`、任何 `config/architecture/*`、`architecture-status.toml`

改动：

- §2 去掉 Executive 框。入口 `aletheon`；Turn/Session/Agent 权威在 `runtime`；主编排在 `aletheon/src/wiring/application`。
- §3 删 `executive`/`fabric` 行。加 `application`（窄纯用例/已接线契约）和 `contracts`。`aletheon` 写成 composition + host wiring。
- §4 `Executive Turn` 改为 daemon `TurnEngine`/`TurnPipeline`；exec 仍走待收敛的 `TurnService`。
- §4.1 `TurnEngine` 路径改为 `crates/aletheon/src/wiring/application/turn_engine.rs`。
- §7 里 `crates/executive/...`、`crates/fabric/...` 改现行路径或标 historical。
- 页头 `Verified` 改为当日。写明 selected `application` 用例已有生产调用，主编排仍在 `wiring/application`。

验证：

```bash
! rg -n 'crates/executive|crates/fabric|^\| `executive`|^\| `fabric`' docs/design/architecture-overview.md
rg -n 'TurnEngine|contracts|application' docs/design/architecture-overview.md
git diff --check -- docs/design/architecture-overview.md
```

停止：总览不再把 Executive/Fabric 写成现行 crate。不改代码。

---

### CC-P0.2 刷新活账本并给未消费 TSV 定性

前置：CC-P0.1 已审过。`architecture-check.sh` **不读** `executive-layers.tsv`、`state-machine-inventory.tsv`。

允许改：

- `architecture-status.toml`
- `config/architecture/persistence-surfaces.tsv`
- `config/architecture/config-ownership.tsv`
- `config/architecture/fabric-public-types.tsv`
- `config/architecture/module-boundaries.txt`
- `config/architecture/hotspot-budgets.tsv`
- `config/architecture/state-machine-inventory.tsv`
- `config/architecture/executive-layers.tsv`

禁止改：`.rs`；不得改 `AppConfig` 字段集合；不得改 workspace crate 名集合。

改动：

- `architecture-status.toml` 的 `from = "executive"` 改为现行 owner 或删除不存在的边。
- `persistence-surfaces.tsv`：仍指向 `executive` 的 owner、schema path、reader、writer 全部改为当前代码事实；保留 schema/version/migration 语义。
- `config-ownership.tsv`：刷新 owner/consumer，不改 `AppConfig` 字段集合或配置 schema。
- `fabric-public-types.tsv`：owner `fabric` → `contracts`；移除 `executive` consumer，按当前生产引用保留/补充真实 consumer，不做全表盲替换。
- `module-boundaries.txt`：用 `bash scripts/cargo-agent.sh metadata --no-deps --format-version 1` 重建完整 workspace-local dependency 列；不要用仅保存未审边的 `architecture-dependencies.txt` 代替。
- `state-machine-inventory.tsv` 定性为 `living / ungated`；`:3` 的 `largest_io_module` 改为 `crates/aletheon/src/wiring/application/turn_pipeline.rs`，`largest_io_loc` 用 `wc -l` 现值。
- `executive-layers.tsv` 定性为 `historical / retired`，只标 header，不把旧快照伪装成现行 owner 图。
- checker 不消费上述两份 TSV；本包不得扩大 scope 修改 checker，也不得把 architecture acceptance 当作它们同步的证据。
- `hotspot-budgets.tsv` 增加且仅增加：

```text
crates/aletheon/src/wiring/application/turn_pipeline.rs	2524	aletheon-turn	daemon turn orchestration
```

验证：

```bash
test -f crates/aletheon/src/wiring/application/turn_pipeline.rs
! rg -n 'from = "executive"|crates/(executive|fabric)' architecture-status.toml
! rg -n '\texecutive\t|crates/(executive|fabric)' config/architecture/persistence-surfaces.tsv
! rg -n '\texecutive\.composition\t|crates/(executive|fabric)' config/architecture/config-ownership.tsv
! rg -n '\|[^|]*fabric[^|]*\||crates/(executive|fabric)' config/architecture/module-boundaries.txt
awk -F'\t' '!/^#/ && ($5 == "fabric" || $6 ~ /(^|,)executive(,|$)/) { bad=1 } END { exit bad }' config/architecture/fabric-public-types.tsv
awk -F'\t' '$1=="crates/aletheon/src/wiring/application/turn_pipeline.rs"{print $2}' config/architecture/hotspot-budgets.tsv
bash scripts/cargo-agent.sh metadata --no-deps --format-version 1 > /tmp/aletheon-architecture-metadata.json
python3 - <<'PY'
import json
from pathlib import Path
metadata = json.loads(Path("/tmp/aletheon-architecture-metadata.json").read_text())
names = {package["name"] for package in metadata["packages"]}
expected = {
    package["name"]: sorted({dependency["name"] for dependency in package["dependencies"] if dependency["name"] in names})
    for package in metadata["packages"]
}
recorded = {}
for line in Path("config/architecture/module-boundaries.txt").read_text().splitlines():
    if not line or line.startswith("#") or line.startswith("ownership|"):
        continue
    crate, _, dependencies, *_ = line.split("|")
    recorded[crate] = sorted(filter(None, dependencies.split(",")))
if expected != recorded:
    raise SystemExit("module-boundaries local_dependencies differ from cargo metadata")
PY
bash scripts/aletheon.sh acceptance architecture
```

停止：活账本不再把 `executive`/`fabric` 写成现行 owner；两份未消费 TSV 已定性。

---

### CC-P0.3 历史文档标成历史

前置：CC-P0.2 已审过。

允许改：

- `docs/arch/CORE_REFACTOR_VERIFICATION_STATUS.md`
- `docs/arch/PUBLIC_API_CONTRACTION_INVENTORY.md`
- `docs/design/roadmap/open-questions.md`
- `docs/arch/README.md`

禁止改：`.rs`、`config/architecture/*`（P0.2 已封）

改动：在仍写“保留 Fabric/Executive 实体 crate”或 `crates/corpus/src/security/loop_detector.rs` 的位置加现状一句：crate 已退役，实现在 `contracts` / `corpus`。不改 Phase 10 当时的数字。

验证：

```bash
rg -n 'retain Fabric and Executive|crates/fabric/src/security/loop_detector' docs/arch docs/design/roadmap/open-questions.md
# 每个命中附近 3 行须有 historical / retired / 现行路径
```

停止：P0 结束。下一包是 CC-P1.1，需人审后领取。

---

### CC-P1.1 写下唯一 Application owner

前置：P0 三包完成。

允许改：

- `crates/application/src/lib.rs` 模块文档
- `docs/design/architecture-overview.md` §3

禁止改：删除 facade（那是 CC-P1.2）；Turn/exec 代码（CC-P1.3）

选定故事（不要并列另一种）：

```text
crates/application          窄纯契约 / 已接线用例
crates/aletheon/src/wiring/application/ host 编排（Turn/Goal/Agent Control）
ApplicationFacade           不是生产入口（下一包删除）
```

验证：

```bash
! rg -n 'ApplicationFacade is the|Session/Turn/Delegate facade' \
  crates/application/src/lib.rs crates/application/src/use_case.rs \
  docs/design/architecture-overview.md
```

---

### CC-P1.2 删除 `ApplicationFacade`

前置：CC-P1.1 已审过。

允许改：

- `crates/application/src/use_case.rs`
- `crates/application/src/lib.rs`

禁止改：`daemon_lifecycle` / `settlement` / `session_input`；`aletheon` 生产路径

先确认：

```bash
rg -n 'ApplicationFacade|DefaultApplicationFacade' crates --glob '*.rs'
```

只删 trait/struct 和仅服务它的 in-crate 测试。

验证：

```bash
! rg -n 'ApplicationFacade|DefaultApplicationFacade' crates --glob '*.rs'
bash scripts/cargo-agent.sh test -p application --lib
```

---

### CC-P1.3 `aletheon exec` 进入 `TurnEngine`

前置：CC-P1.2 已审过。

允许改：

- `crates/aletheon/src/wiring/exec_session.rs`
- `crates/aletheon/src/wiring/composition/turn_service.rs`
- `crates/aletheon/src/wiring/application/turn_engine.rs`
- `crates/aletheon/src/wiring/application/daemon_turn_engine.rs`
- `architecture-status.toml` 的 `TurnService` 行
- `crates/aletheon/tests/turn_service_equivalence.rs`
- `crates/aletheon/tests/exec_cli.rs`
- `crates/aletheon/tests/daemon_turn_api_boundary.rs`（仅当断言必须跟着改）

禁止改：`main.rs` 的 `-m` 路径（已走 daemon）；新建 `SessionTurnEngine`；整文件搬迁 `turn_pipeline.rs`

改动：`ExecSessionBuilder` 经 `TurnEngine` 执行。`TurnService` 若保留，只能薄委托 `TurnEngine`。`architecture-status.toml` 去掉 `internal_compatibility` 或标 `retired`。

验证：

```bash
! rg -n 'TurnService::new' crates/aletheon/src --glob '*.rs'
rg -n 'impl TurnEngine for|TurnEngine::execute' crates/aletheon/src/wiring/exec_session.rs
bash scripts/cargo-agent.sh test -p aletheon --test turn_service_equivalence --test exec_cli --test daemon_turn_api_boundary
```

---

### CC-P2.1 切断 `dasein` → `corpus`

前置：CC-P1.3 已审过。

允许改：

- `crates/dasein/src/bridge/policy.rs`
- `crates/dasein/src/bridge/loop_detector.rs`
- `crates/dasein/Cargo.toml`
- `crates/contracts` 仅当现有契约没有等价 port（先 `rg -n 'PolicyVerdict|LoopVerdict' crates/contracts`）
- `crates/aletheon/src/wiring` 组装注入点

禁止改：放宽默认策略；留下 `corpus` 依赖“备用”

`PolicyBridge` / `LoopBridge` 只依赖 port。默认策略必须由组装注入，且 Verdict 与 `PolicyEngine::with_defaults()` 对照。

验证：

```bash
! rg -n 'use corpus::|corpus =' crates/dasein --glob '*.{rs,toml}'
bash scripts/cargo-agent.sh test -p dasein --lib
bash scripts/cargo-agent.sh check -p aletheon
```

---

### CC-P2.2 切断 cognit 对 runtime 实现类型

前置：CC-P2.1 已审过。

允许改：

- `crates/cognit/src/harness/linear/mod.rs`
- `crates/cognit/src/adapters/inference/pulse.rs`
- `crates/cognit/src/harness/factory.rs`
- `crates/cognit/Cargo.toml`
- `crates/contracts` 或 cognit 自有 port
- `crates/aletheon` 注入点

禁止改：把 `runtime` 加回生产依赖来过测试；一次拿不掉 `tonic` 时假装已切断（在完成报告单列）

验证：

```bash
python3 - <<'PY'
from pathlib import Path
bad = []
for path in Path("crates/cognit/src").rglob("*.rs"):
    production = path.read_text().split("#[cfg(test)]", 1)[0]
    if "use runtime::" in production:
        bad.append(str(path))
if bad:
    raise SystemExit("production runtime imports remain: " + ", ".join(bad))
PY
python3 - <<'PY'
import tomllib
from pathlib import Path
manifest = tomllib.loads(Path("crates/cognit/Cargo.toml").read_text())
bad = sorted({"reqwest", "rusqlite"} & set(manifest.get("dependencies", {})))
if bad:
    raise SystemExit("production dependencies remain: " + ", ".join(bad))
PY
bash scripts/cargo-agent.sh test -p cognit --lib
bash scripts/cargo-agent.sh check -p aletheon
```

---

### CC-P2.3 `mnemosyne` 的 `cognit`/`application` 改为可选

前置：CC-P2.2 已审过。

允许改：`crates/mnemosyne/Cargo.toml` 与因此必须改的 `mnemosyne` 源码 feature gate

禁止改：为编过而写 `default = ["cognitive-memory"]`。daemon 若必须开 feature，停下来改计划。

验证：

```bash
rg -n 'cognit|application' crates/mnemosyne/Cargo.toml
bash scripts/cargo-agent.sh check -p mnemosyne
bash scripts/cargo-agent.sh check -p aletheon
```

---

## 4. 完成报告模板

每个包结束时只交这些：

```text
Slice:
STATUS: DONE | BLOCKED
Files changed:
Verify commands + results:
Locator drift (if any):
Stopped because:
Next slice (do not start):
```

`BLOCKED` 时写清冲突的 `path:line` 和需要人审的那一句计划。
