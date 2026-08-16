# Aletheon 耦合收敛实施计划

目标：账本与代码同树；生产只承认一层 Application 和一条 Turn；砍掉三组高成本耦合。

边界：

- 包含：P0 账本地图、P1 Application/Turn 收敛、P2 三组反向耦合。
- 不包含：Finding F / 原 P3（背压普查、重启语义差、安装态验收）。那些仍是诊断 §8 的证据要求，不是本包步骤。
- 不包含：再拆 `aletheon` 成新的 executive 形 crate；按行数拆 `corpus`/`runtime`；当前分支其余未提交迁移。
- 一次只做一个编号步骤。验证通过后停下等人审。禁止 `git add -A`。
- 本包已获准执行 P0–P2；一次只执行一个已编号切片。Finding F 仍为 `NEEDS EVIDENCE`。
- DeepSeek 按任务包领取，不要把本文件整份丢给模型：[`2026-08-16-architecture-coupling-deepseek-packets.md`](./2026-08-16-architecture-coupling-deepseek-packets.md)

权威诊断：[`docs/arch/evidence/2026-08-16-architecture-coupling-diagnosis.md`](../arch/evidence/2026-08-16-architecture-coupling-diagnosis.md)

相关但不在本包：[`2026-08-15-architecture-hardening.md`](./2026-08-15-architecture-hardening.md) 的锁 poison（hardening P0）与死代码核实（hardening P2）。本包取代 hardening P1 里“按权威整包搬回”的拆分顺序。

```text
P0 地图      ->  P1 一条 Turn / 一层 Application  ->  P2 三组反向耦合
可并行：无。P0 不过，禁止开 P1。
Finding F 证据检查留在诊断 §8，不进入本序列。
```

---

## 前置条件

在仓库根执行。任一条失败则停，先改本计划 locator，不要改代码迁就旧文本。

```bash
test ! -d crates/executive
test ! -d crates/fabric
test -d crates/application
test -f crates/aletheon/src/wiring/application/turn_pipeline.rs
test -f crates/aletheon/src/wiring/composition/turn_service.rs
rg -n 'name = "executive"|name = "fabric"' Cargo.toml crates/*/Cargo.toml && exit 1 || true
```

当前工作区满足这些条件。`origin/dev`（2026-08-16）仍有 `executive`/`fabric`，不能在那棵树上执行 P0 重写。

---

## 实施顺序

### P0.1 重写架构总览

- 文件：`docs/design/architecture-overview.md`
- 改动：
  - §2 运行结构：去掉 `Executive` 框。入口是 `aletheon` composition；Turn/Session/Agent 权威在 `runtime`；编排在 `aletheon/src/wiring/application`。
  - §3 crate 表：删除 `executive`、`fabric` 行。加入 `application`（窄纯用例/契约）、`contracts`（共享契约）。`aletheon` 写成 composition + host wiring，不写成领域 owner。
  - §4 请求流：`Executive Turn` 改为 `aletheon TurnEngine` → `TurnPipeline`（daemon）或待收敛的 `TurnService`（exec）。
  - §4.1 `TurnEngine` 路径改为 `crates/aletheon/src/wiring/application/turn_engine.rs`。
  - §7 未完成项里所有 `crates/executive/...`、`crates/fabric/...` 改成现行路径或标为历史。
  - 页头 `Verified` 改为执行当日。
- 验证：

```bash
rg -n 'crates/executive|crates/fabric|^\| `executive`|^\| `fabric`' docs/design/architecture-overview.md && exit 1 || true
rg -n 'TurnEngine|contracts|application' docs/design/architecture-overview.md
```

### P0.2 刷新 owner 账本

- 文件：
  - `architecture-status.toml`
  - `config/architecture/persistence-surfaces.tsv`
  - `config/architecture/config-ownership.tsv`
  - `config/architecture/fabric-public-types.tsv`
  - `config/architecture/module-boundaries.txt`
  - `config/architecture/hotspot-budgets.tsv`
  - `config/architecture/state-machine-inventory.tsv`
  - `config/architecture/executive-layers.tsv`
- 改动：
  - `architecture-status.toml:63-66`：`from = "executive"` 改为现行集成测试 owner（`aletheon` 或删掉已不存在的边）。
  - `persistence-surfaces.tsv`：把仍指向 `executive` 的 owner、schema path、reader 和 writer 单元格改为当前代码事实；保留现有 schema/version/migration 语义，不得只改 owner 后留下幽灵路径。
  - `config-ownership.tsv`：`executive.composition` 改为当前 owner/consumer；只刷新所有权事实，不改 `AppConfig` 字段集合或配置 schema。
  - `fabric-public-types.tsv`：owner `fabric` → `contracts`；移除 `executive` consumer，并按当前生产引用保留/补充真实 workspace consumer，不做无证据的全表 `executive → aletheon` 替换。
  - `module-boundaries.txt`：从 `bash scripts/cargo-agent.sh metadata --no-deps --format-version 1` 重建每个 workspace package 的 `local_dependencies`；该列记录完整 workspace-local 依赖，不能用只保存未审边的 `architecture-dependencies.txt` 代替。其余列仍按当前目录事实刷新。
  - `state-machine-inventory.tsv` 是 living、但未被 checker 消费的账本：header 标明 `living / ungated`；`:3` 的 `largest_io_module` 改为 `crates/aletheon/src/wiring/application/turn_pipeline.rs`，`largest_io_loc` 更新为当前行数。
  - `executive-layers.tsv` 是已退役 Executive 的历史快照：header 标明 `historical / retired`，不要把整份旧路径映射成新的现行 owner 图。
  - `scripts/libexec/aletheon/architecture-check.sh` **不读**上述两份 TSV；不得把 architecture acceptance 的通过当作它们已同步的证据，也不要在本切片扩大 scope 修改 checker。
  - `hotspot-budgets.tsv` 增加一行，冻结现状，禁止再涨：

```text
crates/aletheon/src/wiring/application/turn_pipeline.rs	2524	aletheon-turn	daemon turn orchestration
```

    现有行不要下调到当前行数以下以外的“优化”；本步只加 `turn_pipeline.rs`。
- 验证：

```bash
! rg -n 'from = "executive"|crates/(executive|fabric)' architecture-status.toml
! rg -n '\texecutive\t|crates/(executive|fabric)' config/architecture/persistence-surfaces.tsv
! rg -n '\texecutive\.composition\t|crates/(executive|fabric)' config/architecture/config-ownership.tsv
! rg -n '\|[^|]*fabric[^|]*\||crates/(executive|fabric)' config/architecture/module-boundaries.txt
awk -F'\t' '!/^#/ && ($5 == "fabric" || $6 ~ /(^|,)executive(,|$)/) { bad=1 } END { exit bad }' config/architecture/fabric-public-types.tsv
# architecture-status 的 legacy freeze key、历史证据文件及文件名可以保留旧词；这里只拒绝现行 owner/path/dependency 字段。
test -f crates/aletheon/src/wiring/application/turn_pipeline.rs
awk -F'\t' '$1=="crates/aletheon/src/wiring/application/turn_pipeline.rs"{print $2}' config/architecture/hotspot-budgets.tsv
bash scripts/cargo-agent.sh metadata --no-deps --format-version 1 > /tmp/aletheon-architecture-metadata.json
python3 - <<'PY'
import json
from pathlib import Path

metadata = json.loads(Path("/tmp/aletheon-architecture-metadata.json").read_text())
names = {package["name"] for package in metadata["packages"]}
expected = {
    package["name"]: sorted({
        dependency["name"]
        for dependency in package["dependencies"]
        if dependency["name"] in names
    })
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

`architecture-check` 用 `module-boundaries.txt` 对 workspace crate 名、用 `config-ownership.tsv` 对 `AppConfig` 字段。改 owner 时不得漏字段、不得改 crate 名集合。

### P0.3 把历史文档标成历史

- 文件：
  - `docs/arch/CORE_REFACTOR_VERIFICATION_STATUS.md`
  - `docs/arch/PUBLIC_API_CONTRACTION_INVENTORY.md`
  - `docs/design/roadmap/open-questions.md`
  - `docs/arch/README.md`
- 改动：在仍写“保留 Fabric/Executive 实体 crate”或 `crates/fabric/src/security/loop_detector.rs` 的位置加一行现状：crate 已退役，实现分别在 `contracts` / `corpus`。不要改写 Phase 10 当时的验收数字。
- 验证：

```bash
rg -n 'retain Fabric and Executive|crates/fabric/src/security/loop_detector' docs/arch docs/design/roadmap/open-questions.md
# 命中行的前后 3 行必须出现 “historical” 或 “retired” 或现行路径。
```

P0 完成标准：`bash scripts/aletheon.sh acceptance architecture` 通过；活账本不再把 `executive`/`fabric` 写成现行 owner。

---

### P1.1 写下唯一 Application owner

选定（不再并列两种故事）：

```text
crates/application          窄纯契约 / 已接线用例
                            DaemonLifecycleService, TransactionReviewService,
                            SessionInputCoordinator, Approval, GoalDraft, ...
aletheon/wiring/application host 编排：TurnPipeline, Goal coordinator,
                            Agent Control, Evaluation, Conscious
ApplicationFacade           删除；不是生产入口
```

- 文件：
  - `crates/application/src/lib.rs` 模块文档（现 `:1-7` 仍把 facade 写成 Session/Turn/Delegate 入口）
  - `docs/design/architecture-overview.md` §3 `application` / `aletheon` 行
- 改动：crate 文档与总览使用同一段 owner 文字。禁止再写“ApplicationFacade 是权威入口”。
- 验证：

```bash
rg -n 'ApplicationFacade is the|Session/Turn/Delegate facade' crates/application/src/lib.rs crates/application/src/use_case.rs docs/design/architecture-overview.md && exit 1 || true
rg -n 'DaemonLifecycleService|SessionInputCoordinator|wiring/application' crates/application/src/lib.rs docs/design/architecture-overview.md
```

### P1.2 删除未使用的 `ApplicationFacade`

- 文件：
  - `crates/application/src/use_case.rs`（`ApplicationFacade`、`DefaultApplicationFacade`、仅被 facade 使用的空 `CreateSession` 等 marker）
  - `crates/application/src/lib.rs` 的 `pub use use_case::{ApplicationFacade, ...}`
- 改动：
  - 先全仓确认无生产调用：`rg -n 'ApplicationFacade|DefaultApplicationFacade' crates --glob '*.rs'`
  - 删除 trait/struct 与仅服务它的 in-crate 测试。
  - 不要删生产在用的 `daemon_lifecycle` / `settlement` / `session_input`。
- 验证：

```bash
rg -n 'ApplicationFacade|DefaultApplicationFacade' crates --glob '*.rs' && exit 1 || true
bash scripts/cargo-agent.sh test -p application --lib
```

### P1.3 `aletheon exec` 进入 `TurnEngine`

- 文件：
  - `crates/aletheon/src/wiring/exec_session.rs`（现 `:258-265` 构造 `TurnService`）
  - `crates/aletheon/src/wiring/composition/turn_service.rs`
  - `crates/aletheon/src/wiring/application/turn_engine.rs`
  - `crates/aletheon/src/wiring/application/daemon_turn_engine.rs`
  - `architecture-status.toml` `TurnService` 行
  - `crates/aletheon/tests/turn_service_equivalence.rs`
  - `crates/aletheon/tests/exec_cli.rs`
- 改动：
  - `ExecSessionBuilder` 构造并调用 `TurnEngine`（可复用 `DaemonTurnEngine` 或同 reducer 的 exec 适配）。禁止再 `TurnService::new` 作为生产编排器。
  - `TurnService` 若还要留着，只能是对 `TurnEngine` 的薄委托，并在 `architecture-status.toml` 把 status 改为 `retired` 或删掉 `internal_compatibility` 行。
  - `converge_into = "SessionTurnEngine"`：不要新建同名类型，除非它就是 `TurnEngine` 的别名。权威名字是已有的 `TurnEngine`。
  - `-m` 已经走 daemon（`main.rs:629-641`），不要改这条路径来“证明”收敛。
- 验证：

```bash
rg -n 'TurnService::new' crates/aletheon/src --glob '*.rs' && exit 1 || true
rg -n 'impl TurnEngine for|TurnEngine::execute' crates/aletheon/src/wiring/exec_session.rs
bash scripts/cargo-agent.sh test -p aletheon --test turn_service_equivalence --test exec_cli --test daemon_turn_api_boundary
```

P1 完成标准：文档只承认一套 Application owner；`ApplicationFacade` 零引用；exec 与 daemon 都经 `TurnEngine`。

---

### P2.1 切断 `dasein` → `corpus`

- 文件：
  - `crates/dasein/src/bridge/policy.rs`（`:2` `use corpus::security::policy`）
  - `crates/dasein/src/bridge/loop_detector.rs`（`:3` `use corpus::security::loop_detector`）
  - `crates/dasein/Cargo.toml`
  - `crates/contracts` 新增窄 port（仅当现有 `contracts` 没有等价 trait；先 `rg -n 'PolicyVerdict|LoopVerdict' crates/contracts`）
  - `crates/aletheon/src/wiring` 组装点（把 corpus 实现注入 dasein bridge）
- 改动：
  - `PolicyBridge` / `LoopBridge` 只依赖 `contracts` 上的 port + `Verdict` 映射。禁止 `PolicyEngine::with_defaults()` 在 dasein 内构造 corpus 实现。
  - 从 `dasein/Cargo.toml` 去掉 `corpus`。若还有别的 `use corpus::`，一并改 port，不要留 `corpus` 依赖“备用”。
- 验证：

```bash
rg -n 'use corpus::|corpus =' crates/dasein --glob '*.{rs,toml}' && exit 1 || true
bash scripts/cargo-agent.sh test -p dasein --lib
bash scripts/cargo-agent.sh check -p aletheon
```

### P2.2 切断 `cognit` → `runtime` 实现类型

- 文件：
  - `crates/cognit/src/harness/linear/mod.rs`（`:64,99` `runtime::compaction`）
  - `crates/cognit/src/adapters/inference/pulse.rs`（`:16` `CanonicalEventBus`）
  - `crates/cognit/src/harness/factory.rs`
  - `crates/cognit/Cargo.toml`（`:11` `runtime`；`:28-33` `reqwest`/`rusqlite`/`tonic`）
- 改动：
  - compaction / event bus / `TurnPolicy` 改为 `contracts` 或 cognit 自有 port；由 `aletheon` 注入 runtime 实现。
  - `reqwest`/`rusqlite` 退出 cognit **生产**依赖。adapter 若仍要 HTTP/DB，放到 `cognit` 已有 `adapters` 且不从 crate root 暴露，或移到 composition 所在 crate。本步若一次拿不掉 `tonic`（policy gRPC），在 PR 说明里单独列出，不要假装已切断。
- 验证：

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

### P2.3 `mnemosyne` 的 `cognit`/`application` 改为可选

- 文件：`crates/mnemosyne/Cargo.toml`（`:10-16` 无条件依赖；`:44-45` `cognitive-memory` 默认关）
- 改动：
  - `cognit` 依赖加上 `optional = true`，只被 `cognitive-memory`（或更窄 feature）打开。
  - `application`：若默认 daemon 只用 `application` 的 DTO，改为 `contracts` 类型；否则同样 optional。
  - 确认 daemon 组装**没有**打开这些 feature。若必须打开，停下来更新本计划，不要为了绿依赖强行关 feature。
- 验证：

```bash
rg -n 'cognit|application' crates/mnemosyne/Cargo.toml
bash scripts/cargo-agent.sh check -p mnemosyne
bash scripts/cargo-agent.sh check -p aletheon
# aletheon 的 Cargo.toml 不得为 mnemosyne 打开刚标 optional 的 cognit，除非计划已改口。
```

P2 完成标准：`dasein` 无 `corpus` 依赖；cognit 生产源无 `use runtime::`；mnemosyne 默认图不链 `cognit`。

Finding F（背压调用方、重启语义差、安装态）不在本包。证据要求见诊断 §8。

---

## 风险与回退

| 步骤 | 风险 | 回退 |
|---|---|---|
| P0.2 | 改 TSV 漏字段导致 `architecture-check` 失败 | 只回该 TSV；不要改 checker |
| P1.2 | 漏掉测试里的 facade 别名 | 删除前 `rg` 必须为零 |
| P1.3 | exec 与 daemon 工具/身份绑定不一致 | 保留 `TurnService` 文件但停用生产构造，按提交回退 `exec_session.rs` |
| P2.1 | SelfField 默认策略变松 | 组装必须注入与 `PolicyEngine::with_defaults()` 等价的 adapter；加 dasein 单测对照旧 Verdict |
| P2.2 | cognit 测试依赖 runtime 类型 | 测试用 fake port，不要把 runtime 加回生产依赖 |
| P2.3 | 默认关 feature 后 daemon 编不过 | 立刻恢复依赖并更新本计划，不要加 `default = ["cognitive-memory"]` 蒙混 |

每步独立提交。提交范围仅限该步文件。

---

## 给 DeepSeek 的第一句话

见 [`2026-08-16-architecture-coupling-deepseek-packets.md`](./2026-08-16-architecture-coupling-deepseek-packets.md) §0。不要把 closeout 全文或诊断全文一次丢进去。
