# Agent Kernel V2 重构 — Codex 验收交接

Date: 2026-08-10
Branch: `fix/tui-live-agent-inspector`
Baseline: `0bf690b2`（含 PR #192 foundation）
Commits: **45**（`git rev-list --count 0bf690b2..HEAD`）
工作树：仅剩在途 TUI 分支未提交文件（`crates/interact/src/tui/*` + `session_projection.rs`）

## 1. 验收目标

Codex 应验证：(a) 全部可加法 owner-seam 层已实现、测试通过、gate 生效；(b) 剩余部署门控 PR-C 与硬件/扩展切流有明确的代码就绪度与执行清单；(c) 重构严格遵循 runbook 的单 writer 协议，无违规。

## 2. 已实现并验证（45 commits）

### Wave A：Phase-0 证据 census（7 项，全部 `CLOSED`）

| Slice | 产物 | 证据报告 |
|---|---|---|
| RA-00 | `runtime-authority-census.tsv`（74 行） | `2026-08-09-ra-00-runtime-authority-census.md` |
| K0 | `kernel-effect-census.tsv`（38 行） | `2026-08-09-k0-kernel-effect-census.md` |
| APX-00 | `application-use-case-census.tsv`（26 行） | `2026-08-09-apx-00-application-io-census.md` |
| CGP-00 | `gateway-route-census.tsv` + `composition-root-census.tsv` | `2026-08-09-cgp-00-composition-gateway-census.md` |
| E0 | `extension-preservation.tsv`（5 扩展） | `2026-08-09-e0-extension-preservation.md` |
| D0 | `fabric-boundary-census.tsv`（1110 符号）+ B2/B4 关闭 | `2026-08-09-d0-fabric-boundary-census.md` |
| XRET-00 | `executive-surface-ledger.tsv`（372 行） | `2026-08-09-xret-00-executive-surface-freeze.md` |

### Wave B：owner seam 建立（8 项）

`D1`(contracts seed) · `RA-01`(Runtime contract) · `K1`(durable Operation) · `K2`(sealed registry) · `CGP-01`(composition skeleton) · `CGP-02`(gateway-protocol + gateway-client) · `APX-01`(Application facade) · `E1`(extension ports)

### Wave C：Runtime 主 writer 链（PR-A seam + PR-B/PR-C 可部署代码）

- `RA-02` journal shadow（只读）· `RA-03` SessionAuthority seam + **PR-C RuntimeSessionWriter** + **PR-B shadow verifier** + **daemon gated wiring**（`session_writer_runtime` flag，默认 legacy）
- `RA-04` Turn reducer seam + **PR-C RuntimeTurnWriter**
- `RA-05` AgentSupervisor seam + **PR-C RuntimeAgentSupervisor**

### Kernel 链（K3-K6）· Gateway（CGP-03/05）· Application（APX-02/03/04）· Domain（D2-D5）· Closure（R1/R2/R3/S1/C1/M1/A1）

每个 slice：`crates/*/src` 新模块 + `config/architecture/*.tsv` census + `scripts/libexec/aletheon/architecture-check.sh` monotonic gate + `docs/arch/evidence/2026-08-*.md` 证据报告。

## 3. 验收命令（全部应为 PASS）

```bash
# 1) 架构 acceptance（含全部 14 个 monotonic gate）
ARCH_SKIP_DEPENDENCIES=1 bash scripts/aletheon.sh acceptance architecture
#   => X1: 22 migrations, 67 acceptance IDs, 1110 Fabric public types; 23 findings, 0 deps

# 2) 全部相关 crate 编译
for c in runtime executive application gateway gateway-client gateway-protocol \
         aletheon kernel fabric agora corpus cognit dasein mnemosyne platform interact; do
  bash scripts/cargo-agent.sh check -p $c || echo "FAIL: $c"
done

# 3) 新模块单元测试（合计 ~137+）
bash scripts/cargo-agent.sh test -p runtime --lib        # 34 passed
bash scripts/cargo-agent.sh test -p application --lib     # 11 passed
bash scripts/cargo-agent.sh test -p kernel --lib          # 69 passed
bash scripts/cargo-agent.sh test -p gateway --lib         # 40 passed
bash scripts/cargo-agent.sh test -p gateway-client        # 4 passed
bash scripts/cargo-agent.sh test -p agora --lib corpus --lib \
  cognit --lib dasein --lib platform --lib aletheon --lib 2>&1 | grep -E "test result"

# 4) fmt + whitespace
bash scripts/cargo-agent.sh fmt --all -- --check
git diff --check
```

## 4. 剩余工作（部署/硬件/在途分支门控）

### 4.1 部署门控 PR-C（需 maintenance window + `sudo bash scripts/aletheon.sh deploy`）

**代码已就绪（gated / 可部署），剩余执行清单按 runbook §14.2：**

| Slice | 代码状态 | 剩余部署步骤 |
|---|---|---|
| RA-03 Session writer 切流 | `RuntimeSessionWriter` + daemon gated wiring（`session_writer_runtime=false` 默认） | maintenance/drain → freeze → flip flag → deploy → SHA-256 比较 → systemd restart 稳定 → 真实 LLM request → crash/restart/cancel drills → rollback binary |
| RA-04 Turn writer 切流 | `RuntimeTurnWriter`（gated wiring 未接入 daemon） | 同上（gated wiring 待接入） |
| RA-05 AgentSupervisor 切流 | `RuntimeAgentSupervisor`（gated wiring 未接入 daemon） | 同上（gated wiring 待接入） |
| CGP-04 socket cutover | CGP-01 composition skeleton + CGP-03 typed handlers | 新 composition 绑 official socket → drain 旧 daemon → installed acceptance |

**执行前置**：environment 已具备 `/usr/bin/aletheon` installed、`aletheon-core.service` running、sudo OK、provider（lejurobot_deepseek）已配。**但每个 writer PR-C 必须独立部署、独立回滚演练，不得合并（plan §12.4）。**

### 4.2 扩展切流（E-series，需 installed equivalence + HIL）

| Slice | 依赖 | 状态 |
|---|---|---|
| E2-K6a Gmail/Google | E1 + APX-02/03 + K6 | 依赖 RA-03/05 PR-C 完成 |
| E3 GBrain | E1 + D4 | 同上 |
| E4-K6b Hardware | E1 + K6 + HIL | HIL gate 未满足（simulator-only） |
| E5-K6c Robot VLA | D2/E4/K6 | HIL gate 未满足 |
| E6-K6d Pi DelegateBackend | RA-05 + E1/K6 | 依赖 RA-05 PR-C；Pi 文件迁移在 E6 独立 PR |

### 4.3 Cleanup/Retirement（caller-zero 门控）

`K7` / `CGP-08` / `D6` / `AK2-25`(fabric→contracts rename) / `XRET-01..05` — 全部依赖 writer cutover 完成后的 caller-zero 证据；`XRET-05` 是唯一删除空 Executive crate 的 PR。

### 4.4 在途分支（U1）

`fix/tui-live-agent-inspector` 的未提交 TUI 改动正是 U1（TUI 实时态收敛）工作包，本批次刻意未触碰。验收时先单独提交/审阅该分支。

## 5. 已知设计决策（供 Codex 复核）

- `runtime::SessionId/TurnId/AgentRunId` 与 `fabric::SessionId/TurnId` 为**刻意不同语义**（`id-collisions.tsv` retain-distinct-semantics，exit RA-03/RA-04）。
- `SESSION_APPEND_WRITERS` 1→2（RA-03 PR-C seam 加入 Runtime writer；legacy 在 RA-06/XRET 删除）。
- `CORE_EXTERNAL_IDENTIFIER_HITS` 21→23（K6 扩展 family 标签，plan §K6 要求）。
- Interact 56→57（CGP-05 ACP typed_client seam）。

## 6. 下一步（建议给 codex 的验收批次）

1. 运行 §3 全部命令确认 PASS。
2. 审阅 §5 的 metric/baseline 变更是否与 plan 一致。
3. 批准后执行 §4.1 的 RA-03 PR-C 部署（一次只做一个 writer）。
4. 在途 TUI 分支单独提交。
