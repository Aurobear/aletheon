# Wiring ownership migration M0 baseline

状态：**COMPLETED — executable ownership/boundary gate enabled; no runtime behavior change**

日期：2026-08-17

批准设计 SHA-256：`c9b927bd7a21a01309d05ae31bbc41ff8024988fb0b3417cf799f3a673f76039`

## 1. Git 与工作区快照

- HEAD：`85704ed3f641419f86dd08c5be8d26c52c64c19b`
- 排除迁移计划和两份审核稿后的 dirty path：34
- 排序 path-list SHA-256：`5fe336cd825f6f3f116215fd080c6349e19114b99fdcb53877d8b7febb98bc6f`
- 实施开始前 tracked diff SHA-256：`f35d49b1e6dd57e50cd7b14a5d9e2017f87438d442a404079ae9aec9632bee12`

该快照包含用户已有的未提交和未跟踪改动。迁移不得把 HEAD 当作唯一事实源，也不得
回退下列路径：

```text
.agents/
Cargo.lock
config/architecture-allowlist.txt
config/architecture-dependencies.txt
config/architecture/kernel-effect-census.tsv
config/default.toml
config/schema/aletheon-config.schema.json
crates/aletheon/src/main.rs
crates/aletheon/src/wiring/adapters/context_memory.rs
crates/aletheon/src/wiring/adapters/gbrain/mcp_adapter.rs
crates/aletheon/src/wiring/adapters/mod.rs
crates/aletheon/src/wiring/adapters/verification_command.rs
crates/aletheon/src/wiring/adapters/worktree.rs
crates/aletheon/src/wiring/application/approval/apply_coordinator.rs
crates/aletheon/src/wiring/application/approval/mod.rs
crates/aletheon/src/wiring/application/context_assembler.rs
crates/aletheon/src/wiring/application/verification/command.rs
crates/aletheon/src/wiring/application/verification/mod.rs
crates/aletheon/src/wiring/daemon/bootstrap/extensions.rs
crates/aletheon/src/wiring/daemon/bootstrap/memory.rs
crates/aletheon/src/wiring/daemon/bootstrap/request.rs
crates/aletheon/src/wiring/daemon/bootstrap/services.rs
crates/aletheon/src/wiring/doctor.rs
crates/dasein/Cargo.toml
crates/dasein/src/impl/perception/manager.rs
crates/dasein/src/impl/perception/sources/journald_source.rs
crates/mnemosyne/src/memory_gateway.rs
crates/mnemosyne/src/supplemental_memory.rs
crates/platform/src/journald.rs
crates/platform/src/lib.rs

scripts/lib/aletheon/build.sh
scripts/lib/aletheon/install.sh
scripts/libexec/aletheon/install-systemd.sh
```

重算命令：

```bash
git rev-parse HEAD
git status --porcelain=v1 -z
git diff -- . | sha256sum
```

path-list digest 使用 C 排序、每行一个路径且末尾保留换行；排除的三个文件为迁移计划、
完整 Grok review 和 residual review。

## 2. Cargo resolve graph

命令：

```bash
bash scripts/cargo-agent.sh metadata --format-version 1 > /tmp/aletheon-m0-metadata.json
```

结果：

- workspace package：20
- workspace-local dependency edge：80
- metadata JSON bytes：2,175,847
- metadata SHA-256：`85f8fdb24ddce6c6e48394256cf64c68d265dd4271a3f860a7d382aaa412f239`

该文件是完整 resolve graph，不是 `--no-deps` 摘要。每个修改 Cargo manifest 的 packet
必须重新生成并检查禁止回边。

## 3. Wiring 规模与 ownership

```text
crates/aletheon/src/wiring/application  69 files  29,329 LOC
crates/aletheon/src/wiring/adapters     45 files  16,846 LOC
crates/aletheon/src/wiring/daemon       65 files  22,724 LOC
```

统计命令：

```bash
python3 - <<'PY'
from pathlib import Path
for root in [Path('crates/aletheon/src/wiring/application'),
             Path('crates/aletheon/src/wiring/adapters'),
             Path('crates/aletheon/src/wiring/daemon')]:
    files = list(root.rglob('*.rs'))
    print(root, len(files), sum(len(p.read_text().splitlines()) for p in files))
PY
```

所有顶层 wiring 路径的 current/policy/authority/adapter/host owner 和 packet 已冻结在
`config/architecture/wiring-ownership.tsv`。该账本必须与实际顶层路径形成精确集合；迁移
只能把行推进到 final disposition，不得新增未分配路径。

- 顶层路径数：20
- ledger SHA-256：`c72b0c80aa5739150b04a2787438ba4e29738f8f4bd7d668d6603dc722d47d6c`
- 集合检查：实际顶层 Rust 文件/目录与 ledger `source_path` 精确相等

## 4. Wire、持久化和 runtime event parity

M0 发现并修正了四个历史 locator：ConsciousCore protocol、Core RPC、contracts IPC 和
Session projection contract。修正仅更新 inventory，不改变 schema 或运行行为。

| 基线 | SHA-256 |
|---|---|
| `config/architecture/wire-surfaces.tsv` | `74c8f10d8c196cfd1e7b0b75c5dbb0fb0fa18b48b73e9d6d70aacdecb606815a` |
| 22 个现存 wire source 的 path+content aggregate | `f40e476cb149f1df28a5414a3eadf1b5f34f538fdb882e8894a61a7de6662f35` |
| `config/architecture/persistence-surfaces.tsv` | `7dac09131ee8ed5605f9737e89c93747fac1e95591a94d41f6176f5456dd6835` |
| 19 个 persistence source 的 path+content aggregate | `8d2e3575f501252f9653ccaed5b24da517da89227295f1a20451975789b6e50f` |
| `config/architecture/runtime-authority-census.tsv` | `70ff37a26de80c3ed12fa1d505b58e9b7bd0a26302032bac5bd8c5bca62b951f` |
| runtime `event.rs`/`event_spine.rs`/`journal.rs`/`read_model.rs` aggregate | `adb263e5a7d1b7e3e3d7ded7dfbe96f3684c9a81f8a6c5bee3aabd90de01fb26` |
| `config/architecture/kernel-effect-census.tsv` | `eaed09400ae09bda5e9256567ab3707ce2fc885ba4189bee6560261f5ecf008b` |

Aggregate 算法按相对路径排序，并依次 hash：`path + NUL + content + NUL`。物理迁移会改变
path aggregate，因此后续 packet 必须同时证明 schema/version/behavior parity，不能只比较
路径 hash。

## 5. M0 gate 结果

```text
Phase 0 architecture fixtures: pass
architecture-check fixture: pass
X1 contract negative fixtures: pass
multi-user runtime architecture boundary: pass
approval closure matrix verified: closed=4 denied=6
hotspot ownership budgets verified: 7
architecture path inventory: pass
X1 contract gates: 22 migrations, 67 acceptance IDs, 679 Fabric public types
architecture-check: 0 findings, 46 dependencies, 4 paths; no additions
```

执行命令：`bash scripts/aletheon.sh test architecture`。

追加授权后，M0 已修改 architecture checker 及其 fixture：ledger header/列数、顶层路径
全集、唯一 owner、packet/evidence locator 与 §4.2 五项指标均成为持久 gate；fixture 同时
覆盖未登记路径、未决 owner、Application process effect、反向 Aletheon import、Aletheon
concrete repository 和公开 wiring export。上述 architecture 输出为启用新规则后的结果。
