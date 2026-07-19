# Platform Backend Convergence Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让唯一 `platform` crate 的 selector、capability probe 与 Host contracts 使用真实原生 backend，并逐步替代 Corpus 中重复宿主实现。

**Architecture:** Stable contracts 不泄漏 OS handle；selector 每次只选择一个原生 backend；permission 由上层授予，Platform 只执行请求并返回 bounded receipt。每个 OS 必须在原生 runner 验证，跨平台 stub 不算完成。

**Tech Stack:** Rust、Linux native APIs、Windows/macOS conditional compilation、现有 Platform traits。

---

## 需求锚点

- 当前 selector 对所有 OS 返回 stub：`docs/arch/aletheon-host-platform-plan.md:41-48`，代码为 `crates/platform/src/selector.rs:26-33`。
- 完整实施顺序：`docs/arch/aletheon-host-platform-plan.md:68-76`。
- 验收语义：`docs/arch/aletheon-host-platform-plan.md:89-96`。

### Task 1: 接通当前 OS 的真实 capability probe

**Files:**
- Modify: `crates/platform/src/selector.rs`
- Modify: `crates/platform/src/backend/linux/probe.rs`
- Test: `crates/platform/src/selector.rs`
- Update: `architecture-status.toml`

- [x] Linux selector 返回 `backend::linux::LinuxBackend`；对应 target 的 Windows/macOS 分支返回各自原生 probe；Unknown fail closed。
- [x] 当前 Linux 测试要求 platform=`linux`、OS version 非 `stub`、timestamp 非零且 feature states 不全 Unsupported。
- [x] 不把未经探测的 Landlock/seccomp/namespace 直接声明 Available；区分 Available、Unavailable、PermissionRequired、Degraded、Unsupported。
- [x] 运行 `bash scripts/cargo-agent.sh test -p platform` 与 architecture check。
- [x] **确认点:** Linux 已完成；Windows/macOS 只有 cfg 接线，不声称已经原生验证。

### Task 2: 建立共享 contract conformance suite

**Files:**
- Create: `crates/platform/tests/contract_suite.rs`
- Modify: `crates/platform/src/backend/linux/*.rs`（按失败测试逐项）

- [x] `contract_suite` 覆盖 filesystem scope/atomic write、process spawn/timeout/tree cleanup、PTY resize/close、service typed error、sandbox observed strength 与 bounded receipt。
- [x] 行为断言只定义一次并通过当前 native backend factory 构造；Windows/macOS factory 留待原生实现，不复制断言。
- [x] 资源测试使用临时目录与短 timeout，并显式清理 process group/PTY。
- [x] `bash scripts/cargo-agent.sh test -p platform --test contract_suite` 通过（19 passed）。

#### Task 2A: Operation-scoped Filesystem authority

**Files:**
- Modify: `crates/platform/src/filesystem.rs`
- Modify: `crates/platform/src/backend/linux/filesystem_host.rs`
- Test: `crates/platform/tests/linux_contract.rs`
- Later integration: `crates/executive` composition and `crates/corpus/src/tools/tools/file_{read,write}.rs`

- [x] 定义 Platform 自有的执行投影 `FilesystemScope`，包含 admitted roots、read/write access 与 symlink policy；它不是新的权限 authority。
- [x] 删除 Linux backend 的 unrestricted 构造方式，要求每个实例绑定 scope。
- [x] 测试拒绝 scope 外路径、symlink escape 与 read-only write。
- [x] 原子写使用唯一临时文件、文件 sync、rename 和父目录 sync，并落实 mode 与 missing-file precondition。
- [x] Linux root/read handles 在 scope 构造时固定；后续 read/metadata/create/write/remove 使用 `openat2(RESOLVE_BENEATH|RESOLVE_NO_MAGICLINKS)` 与 pinned directory fd，Deny policy 追加 `RESOLVE_NO_SYMLINKS`；不支持 `openat2` 时 fail closed。
- [x] Executive 从 `WorkspacePolicy` 构造请求 scope，Kernel Permit 的 granted scope 随调用 authority 传入 Corpus；Platform 不依赖 Fabric/Kernel 类型。
- [x] Corpus file read/write 只通过 operation-scoped Platform handle 访问文件，保留 protected-path 拒绝规则；filesystem Permit 的空路径集合在此边界 fail closed。
- [x] `apply_patch` 迁移到相同的 operation-scoped contract；统一 diff 与 structured patch 均通过 scoped Platform read/atomic-write/remove，修改和删除使用内容 hash precondition，且保留现有路径约束。
- [x] execd patch 迁移到相同的 operation-scoped contract，并保留 workspace、symlink、profile deny 与 hash precondition 约束。
- [x] **确认点:** 已展示跨层迁移模型并取得用户确认；file read/write 迁移完成并通过窄测试。

### Task 3: 删除重复 stub 语义

**Files:**
- Modify: `crates/platform/src/selector.rs`
- Modify: `crates/platform/src/backend/linux/stub.rs`
- Modify: `crates/platform/src/backend/windows/stub.rs`
- Modify: `crates/platform/src/backend/macos/stub.rs`

- [x] selector 只选择当前编译目标的 native backend，不再维护 Linux/Windows/macOS 三份 feature stub manifest。
- [x] 非目标 OS compile stub 留在各 backend 模块且只报告 Unsupported；目标 OS selector 只使用 native backend。
- [x] Unknown target 只有一个 fail-closed Unsupported backend。

### Task 4: Linux 完整 contract

**Files:**
- Modify: `crates/platform/src/backend/linux/filesystem_host.rs`
- Modify: `crates/platform/src/backend/linux/process_host.rs`
- Modify: `crates/platform/src/backend/linux/pty_host.rs`
- Modify: `crates/platform/src/backend/linux/service_host.rs`
- Modify: `crates/platform/src/backend/linux/sandbox_host.rs`
- Test: `crates/platform/tests/contract_suite.rs`

- [x] filesystem 用 openat2/directory-fd 防止 root escape/symlink race；process receipt bounded 且 cancel 清理进程树；PTY 支持 resize/EOF；service 区分 systemd 缺失与权限拒绝；sandbox 报告实际 strength。
- [x] 当前 Linux contract suite 19 项通过；无法在本环境验证或尚未实现的 capability 记录为 Degraded/Unavailable，未伪造 Available。

### Task 5: Windows 原生 runner

**Files:**
- Modify: `crates/platform/src/backend/windows/*.rs`
- Modify: `.github/workflows/ci.yml`

- [ ] 在 `.github/workflows/ci.yml` 新增 `windows-latest` 原生 job，通过 `bash scripts/cargo-agent.sh test -p platform --test contract_suite` 验证 Job Object 进程树、ConPTY、SCM 与 filesystem；当前 CI 只有 Ubuntu job，不能借用 release cross-build 充当验证。
- [ ] 只有原生 runner 通过后才把 Windows 状态从 stub/unverified 改为 wired。
- [ ] **确认点:** 若仓库没有 Windows runner，保留未完成状态并向用户报告，不用 Linux stub 代替证据。

### Task 6: macOS 原生 runner

**Files:**
- Modify: `crates/platform/src/backend/macos/*.rs`
- Modify: `.github/workflows/ci.yml`

- [ ] 在 `.github/workflows/ci.yml` 新增 `macos-latest` 原生 job，通过 `bash scripts/cargo-agent.sh test -p platform --test contract_suite` 验证 process、PTY、launchd、filesystem/FSEvents/TCC 状态。
- [ ] 只有原生 runner 通过后才标记 wired。
- [ ] **确认点:** 若没有 macOS runner，处理方式同 Windows。

### Task 7: Corpus Host 实现迁移

**Files:**
- Inspect: `crates/corpus/src/drivers/platform/`
- Inspect: `crates/corpus/src/security/sandbox/`
- Modify: 按 caller 清单逐项迁移的 Corpus 文件
- Test: 对应 Corpus integration tests

- [x] caller 审计：Corpus `drivers/platform`（service/process/system info）整树无生产 caller；ContainerBackend 仅 re-export 无 caller；Bubblewrap/Process sandbox 有 bash/runner/Pi caller；X11 clipboard 有 ACI factory caller。
- [x] 删除无 caller 的 Corpus platform adapter 整树与 ContainerBackend；保留语义不同且有生产 caller 的 Fabric argv sandbox 和 clipboard，不强行塞入尚未实现的 Platform sandbox/desktop contract。
- [x] filesystem read/write/patch 已经通过 Platform trait；本轮删除后 `check -p corpus` 与 sandbox 定向测试（34 passed）通过。
- [x] **裁决:** 用户已授权按生产 caller 自主收敛；无 caller 删除，有 caller 且 contract 语义不同则保留并记录。

### Task 8: Execd 使用最小 Platform/patch contract

**Files:**
- Modify: `crates/execd/Cargo.toml`
- Modify: `crates/execd/src/filesystem.rs`
- Test: `crates/execd/tests/protocol_integration.rs`

- [x] 采用 boundary convergence plan 裁决的 Platform patch owner。
- [x] Execd 不依赖完整 Corpus 工具域；patch 操作仍受 confined root、symlink 和 precondition hash 约束。
- [x] 运行 `bash scripts/cargo-agent.sh test -p execd` 与 Platform tests；architecture check 在状态账本更新后运行。

## 完成条件

- [x] `platform::probe()` 在当前 Linux 返回真实版本和观测 feature state。
- [ ] 三 OS 使用同一 contract suite，只有原生 runner 结果算验证。
- [x] unsupported、permission denied、not found 可区分，receipt detail bounded 为 4096 bytes。
- [x] Platform 不反向依赖 Corpus/Executive。
- [x] 不存在 `platform-api/platform-host/platform-linux/...` package。

## 外部验证门禁

- 当前环境是 Linux，没有 Windows/macOS native runner。对应 backend 保持
  Unavailable/unverified；不得用 cfg stub、cross-build 或 Linux 测试替代 Job Object、
  ConPTY、SCM、launchd、FSEvents 与 TCC 的原生运行证据。
- 因原生实现尚未可验证，本轮不向 CI 增加必然失败的伪 job；上述 Task 5/6 与三 OS
  completion checkbox 保持未完成，直到具备对应 runner 和真实 backend。
