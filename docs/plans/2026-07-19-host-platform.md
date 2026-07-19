# Host Platform 多操作系统生产化 Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 Aletheon 从"只在 Linux 靠 shell 命令跑起来"演进为一个受治理的跨操作系统宿主 Runtime。新建 `platform-api`（稳定 trait/错误/capability manifest）+ `platform-host`（backend 选择 + operation receipts）+ `platform-linux`（Linux 生产实现）。上层（Executive/Corpus/Workspace Tools/Pi Runtime/Dasein）只依赖能力与语义，不再直接 `Command::new("systemctl")` / `tokio::fs` / `walkdir`。最终成功标准不是"三个 OS 都能编译"，而是同一个受治理的 Agent operation 在 Linux/Windows/macOS 上具有一致的生命周期、权限、证据与失败语义。

**Architecture:**
```text
Executive / Corpus / Capability Runtime / Workspace Tools / Dasein
                         │  (只依赖 platform-api 契约)
                         ▼
                 Kernel Capability Broker
                         │
                         ▼
                   platform-api          (稳定类型/trait/错误/manifest/probe)
                         │
                   platform-host          (backend 选择 · policy 桥接 · operation receipts)
            ┌────────────┼────────────┐
            ▼            ▼            ▼
     platform-linux platform-windows platform-macos   (H2/H3 deferred)
            │            │            │
       Linux APIs      Win32 APIs   Darwin/Cocoa APIs
```
现有 `crates/corpus/src/drivers/platform/*` 保留 **compatibility facade**：`PlatformAdapter` trait/`create_platform_adapter` 签名不变，内部改为委托 `platform-host`，加 deprecation 告警与删除期限。迁移完成后再删。

**Tech Stack:** Rust；Linux 首发（cgroup v2 / pidfd / inotify / Unix PTY / systemd D-Bus via `zbus`）；后续 Windows（Win32 `CreateProcessW` / Job Object / ConPTY / SCM / `ReadDirectoryChangesW`）、macOS（`posix_spawn` / launchd / FSEvents / Keychain）。

**环境说明:** cargo 可用；构建/测试一律走 `bash scripts/cargo-agent.sh test -p <crate> <filter>`（该脚本已存在，统一 target dir + build lock），**不要用裸 cargo**。OS 契约测试需完整 Rust 环境 + per-OS CI runner，本机只能跑 Linux 分支。

**依赖:** Wave 0（架构冻结门禁：`architecture-status.toml` + `scripts/architecture-check.sh`，禁止新增 OS 特判与直接 process/fs 调用）。H1 与 coding-agent 线 Wave 2 并行——Wave 2 的"Dasein/Executive 命令走 Host Capability"（`crates/dasein/src/impl/security/rollback/mod.rs:459-661` 的 btrfs/systemctl 直调）**依赖 H1 的 ProcessHost/ServiceHost**。H2+（Windows/macOS）押到 coding 线 Wave 5 之后，不阻塞 Linux 生产化。工期：单人 + 代理下 H1 预留 6–8 周当量（原 arch 文档 4 周偏乐观）。

**当前代码事实（已逐行核对 `dev`）:**
- `crates/corpus/src/drivers/platform/adapter.rs:39` — `PlatformAdapter` trait 仅有 service/host-info/privilege，**无** process/fs/pty/net。
- `crates/corpus/src/drivers/platform/mod.rs:17` `create_platform_adapter` + `:35` `BasicLinuxAdapter`，`:57` 直接 `tokio::process::Command::new("systemctl")`。
- `crates/corpus/src/drivers/platform/linux.rs:33` `LinuxPlatformAdapter`，`:53` systemctl shell（伪 D-Bus：持 `Connection` 却仍调命令），`:177` `elevate_privileges` = pkexec/sudo 黑盒。
- `create_platform_adapter` **当前无任何生产调用者**（`grep -rn create_platform_adapter crates` 仅命中定义）→ facade 迁移风险低，可放心先建契约。
- 现有 driver：`display/{x11,clipboard,clipboard_x11,drm,window,window_x11}.rs`、`input/uinput.rs`、`a11y/atspi.rs`、`sandbox_driver/mod.rs`（已有 seccomp/cgroup/namespace 原语）；`proc/mod.rs` 与 `io/mod.rs` 仍是 `TODO: Phase 7/8` 占位。
- 直接 process/fs 泛滥点：`crates/corpus/src/tools/tools/{bash_exec,grep,file_search,process_list,apply_patch,script_tool}.rs`、`crates/dasein/src/impl/security/rollback/mod.rs`、`crates/exec-server/Cargo.toml:15`（依赖整个 corpus）。

---

## H0 — 冻结边界与契约（前置，无 OS 依赖，可全程在 Linux 本机做）

新建两个纯契约/编排 crate + facade + 跨平台编译门禁。此阶段**不写任何真实 OS 系统调用**，只定义稳定类型与 no-op/probe 骨架，保证三 OS 都能 `cargo check`。

### H0-1 新建 `platform-api` crate 骨架

- [ ] 建 `crates/platform-api/Cargo.toml`（deps: `async-trait`, `serde`, `thiserror`, `bytes`；无 OS-specific dep）
- [ ] 加入 workspace：编辑 `Cargo.toml:3` `members` 数组追加 `"crates/platform-api"`, `"crates/platform-host"`, `"crates/platform-linux"`
- [ ] 建 `crates/platform-api/src/lib.rs`：`pub mod error; pub mod path; pub mod context; pub mod manifest; pub mod process; pub mod fs; pub mod pty; pub mod service; pub mod sandbox; pub mod net; pub mod credential; pub mod session; pub mod desktop; pub mod media; pub mod receipt;`

**Files:** `crates/platform-api/Cargo.toml`, `crates/platform-api/src/lib.rs`, `Cargo.toml`

### H0-2 稳定错误、路径、context、receipt

- [ ] `crates/platform-api/src/error.rs`：`HostError { kind: HostErrorKind, native_code: Option<i64>, message: String, diagnostics: Vec<(String,String)> }`；`HostErrorKind` 枚举稳定映射（`NotFound / PermissionDenied / Unsupported / Degraded / Timeout / ResourceExhausted / Conflict / Cancelled / Io / Backend`）。禁止直接暴露 `anyhow::Error`。
- [ ] `crates/platform-api/src/path.rs`：`HostPath { logical: String, native: OsString }` — 逻辑路径 + 保留 native path，**不得**把 Windows 路径硬转 Unix 字符串（源 §4.2）。
- [ ] `crates/platform-api/src/context.rs`：`OperationContext { actor, workspace, capability, policy_decision, deadline: Option<Instant>, trace_id }`（源 §4.2，每次敏感操作携带）。
- [ ] `crates/platform-api/src/receipt.rs`：`OperationReceipt { operation_id, started_at, finished_at, outcome, resource_usage, artifacts: Vec<ArtifactRef> }` — API 返回 receipt，**不能只返回 `bool`**（源 §4.2）。

**Files:** `crates/platform-api/src/{error,path,context,receipt}.rs`

### H0-3 小 trait 定义（直接复用源文档 §4.1，不做巨型 adapter）

- [ ] `crates/platform-api/src/process.rs`：
  ```rust
  #[async_trait] pub trait ProcessHost: Send + Sync {
      async fn spawn(&self, spec: SpawnSpec) -> Result<ProcessHandle, HostError>;
      async fn inspect(&self, id: ProcessId) -> Result<ProcessSnapshot, HostError>;
      async fn signal(&self, id: ProcessId, signal: ProcessSignal) -> Result<(), HostError>;
      async fn terminate_tree(&self, id: ProcessId, grace: Duration) -> Result<ExitStatus, HostError>;
  }
  ```
  `SpawnSpec` 用 `argv: Vec<OsString>`（**不默认拼 shell string**）、`env`、`cwd: HostPath`、`limits: ResourceLimits`、`stdio policy`。
- [ ] `crates/platform-api/src/fs.rs`：`FilesystemHost { metadata / atomic_write(AtomicWrite{ expected_hash, fsync_policy, tmp+rename }) -> WriteReceipt / watch(WatchRequest) -> EventStream<FsEvent> }`（源 §4.1）。文本输出显式携带 encoding + 截断状态 + Artifact 引用。
- [ ] `crates/platform-api/src/pty.rs`：`PtyHost`（open/resize/read/write，窗口尺寸 + 信号 + 原始字节）。
- [ ] `crates/platform-api/src/service.rs`：`ServiceHost`（install/start/stop/restart/status，返回结构化状态非解析文本）。
- [ ] `crates/platform-api/src/sandbox.rs`：`SandboxHost` + `IsolationStrength` + `SandboxLevel { L0 Observe / L1 Workspace / L2 Networked / L3 Desktop / L4 Admin }`（源 §8.2）；API 必须返回缺失保证，不得声称各 OS 等价。
- [ ] `crates/platform-api/src/{net,credential,session,desktop,media}.rs`：`NetworkHost / CredentialHost / UserSessionHost / DesktopHost / MediaHost` marker trait + 关键方法签名（H0 只定义，实现留 H1/H4）。

**Files:** `crates/platform-api/src/{process,fs,pty,service,sandbox,net,credential,session,desktop,media}.rs`

### H0-4 Capability manifest + runtime probe（源 §4.3）

- [ ] `crates/platform-api/src/manifest.rs`：
  ```rust
  pub struct HostCapabilityManifest {
      pub platform: HostPlatform, pub os_version: String, pub arch: Architecture,
      pub backend_version: Version, pub features: BTreeMap<HostFeature, FeatureState>,
      pub constraints: Vec<HostConstraint>, pub probed_at: SystemTime,
  }
  pub enum FeatureState { Available, Unavailable, PermissionRequired, Degraded, Unsupported }
  ```
- [ ] 定义 `trait HostBackend { fn probe(&self) -> HostCapabilityManifest; }` — 编译成功 ≠ 能力可用，启动时必须 runtime probe（源 §4.3）。

**Files:** `crates/platform-api/src/manifest.rs`

### H0-5 `platform-host` backend 选择 + receipts

- [ ] 建 `crates/platform-host/Cargo.toml`（dep `platform-api`；`#[cfg(target_os)]` 门控 `platform-linux/windows/macos`，H0 只接 linux stub）
- [ ] `crates/platform-host/src/lib.rs`：`HostPlatform::select() -> Arc<dyn HostBackend>`（按 `cfg!(target_os)` 选后端）；operation receipt 包装层（统一 operation_id/trace/audit 收据，源 §2.2 缺可观测性）。
- [ ] policy 桥接骨架：`OperationContext` → capability grant 校验挂钩点（`host.process.spawn:workspace` 等，源 §8.1；H0 留接口，H1 接 Kernel Broker）。

**Files:** `crates/platform-host/Cargo.toml`, `crates/platform-host/src/lib.rs`

### H0-6 Compatibility facade（保留旧 `PlatformAdapter`）

- [ ] 保持 `crates/corpus/src/drivers/platform/adapter.rs:39` `PlatformAdapter` trait 签名不变；`crates/corpus/src/drivers/platform/mod.rs:17` `create_platform_adapter` 内部改为构造一个委托 `platform-host` 的 adapter。
- [ ] 对旧 trait 方法加 `#[deprecated(note = "migrate to platform-api ServiceHost/HostInfo; removal target: H1 完成")]`；facade 内部把 `service_*` 映射到 `platform-host` 的 `ServiceHost`，`elevate_privileges` 标记为不再新增调用者。
- [ ] 在 `corpus/Cargo.toml` 加 `platform-host` 依赖。（注意：`create_platform_adapter` 当前无生产调用者，facade 主要是防止未来回归，风险低。）

**Files:** `crates/corpus/src/drivers/platform/mod.rs`, `crates/corpus/src/drivers/platform/adapter.rs`, `crates/corpus/Cargo.toml`

### H0-7 跨平台编译 CI + 冻结门禁

- [ ] 新建/扩展 CI：`platform-api` + `platform-host` 在 `x86_64-unknown-linux-gnu` / `x86_64-pc-windows-msvc` / `aarch64-apple-darwin` 三 target 上 `cargo check`（源 §9.2；注意 §9.2 明确"不能以 `cargo check --all-targets` 代替平台可用性验证" → 这一步只保证编译，不冒充能力验证）。
- [ ] 扩展 `scripts/architecture-check.sh`：门禁新增"禁止 `crates/{executive,corpus,dasein,kernel}` 新增 `Command::new("systemctl"|"btrfs"|...)` 与新增 `#[cfg(target_os)]` OS 特判"（Wave 0b 冻结账本联动，源总纲 §8.1「Host 直接 process/fs 调用 allowlist 只减不增」）。
- [ ] `architecture-status.toml`：为每个新 trait 标 owner=`platform-api` / 生产调用者 / authority / 兼容删除期限。

**Files:** `.github/workflows/*` (host matrix job), `scripts/architecture-check.sh`, `architecture-status.toml`

**H0 验收（源 §10 H0）：** Corpus/Kernel 不再新增 OS 特判；旧 `PlatformAdapter` API 有迁移 deprecation 告警；`platform-api`+`platform-host` 在三 OS target 全部 `cargo check` 通过；契约类型（error/path/context/receipt/manifest/小 trait）冻结成文。测试：`bash scripts/cargo-agent.sh test -p platform-api`（纯类型/序列化单测，本机可跑）。

---

## H1 — Linux 核心生产化（关键路径，coding 线 Wave 2 前置）

在 `platform-linux` 落地真实 Linux 系统调用，替换伪抽象，让 Workspace Tools + Pi Runtime + Dasein 命令统一走 Host capability。这是 CLI Coding Agent 生产化的实际前置。

### H1-1 `platform-linux` crate + backend probe

- [ ] 建 `crates/platform-linux/Cargo.toml`（deps: `platform-api`, `nix`/`rustix`（pidfd/openat/signal）, `zbus`（systemd D-Bus）, `inotify`, `libc`；`[target.'cfg(target_os="linux")'.dependencies]` 门控）
- [ ] `crates/platform-linux/src/lib.rs`：`LinuxBackend` 实现 `HostBackend::probe()` — 探测 cgroup v2 挂载、pidfd、systemd D-Bus socket（对照旧 `linux.rs:28` 的 `/var/run/dbus/system_bus_socket` 检测）、Landlock、seccomp，逐项映射 `FeatureState`（`Available/Degraded/Unsupported`）。

**Files:** `crates/platform-linux/Cargo.toml`, `crates/platform-linux/src/lib.rs`

### H1-2 ProcessHost（Linux 生产，源 §5.1）

- [ ] `crates/platform-linux/src/process.rs`：`spawn` 用 process group + pidfd（可用时，否则降级并在 receipt 标注）；`argv` 直传不拼 shell；stdout/stderr backpressure + 截断 + Artifact 引用；退出状态结构化。
- [ ] `terminate_tree`：先 graceful signal 再强制 kill 整个进程树（源 §4.2「以进程树为单位，超时先 graceful 再强制」）；`grace: Duration` 超时语义。
- [ ] 资源限制：优先 cgroup v2（CPU/mem/pids/io policy），不可用降级 rlimit 并在 manifest/receipt **明确报告降级**（源 §5.1，禁止 silent fallback §9.3）。

**Files:** `crates/platform-linux/src/process.rs`

### H1-3 FilesystemHost + confinement + watcher（源 §5.1）

- [ ] `crates/platform-linux/src/fs.rs`：`openat` 风格目录相对访问，避免"检查路径→再打开"的 TOCTOU；`atomic_write` = tmpfile + fsync policy + rename，支持 `expected_hash`。
- [ ] workspace confinement：symlink 逃逸拒绝（对照现有 `crates/corpus/src/tools/tools/` 的相对路径/symlink 处理，统一到此）。
- [ ] `watch`：inotify 为默认 backend，返回 `EventStream<FsEvent>`，处理重复/乱序/溢出/重建索引（源 §9.1）；高权限审计 backend 不混入默认 Agent 安装（源 §5.1）。

**Files:** `crates/platform-linux/src/fs.rs`

### H1-4 PtyHost（Unix PTY）

- [ ] `crates/platform-linux/src/pty.rs`：Unix PTY open/resize/信号透传/UTF-8 与原始字节流（源 §5.1）。

**Files:** `crates/platform-linux/src/pty.rs`

### H1-5 ServiceHost（真 systemd D-Bus，替换 shell 伪抽象）

- [ ] `crates/platform-linux/src/service.rs`：用 `zbus` 直连 `org.freedesktop.systemd1` 做 list/start/stop/restart/status，**替换** `linux.rs:53`/`mod.rs:57` 的 `systemctl` 文本解析（源 §2.2 问题4「伪抽象」）。
- [ ] 无 systemd（容器/WSL/最小发行版）：返回 `Unsupported` 或走显式配置的 supervisor backend，不 silent fallback（源 §5.1 + §9.3）。

**Files:** `crates/platform-linux/src/service.rs`

### H1-6 SandboxHost（复用现有 sandbox_driver 原语）

- [ ] `crates/platform-linux/src/sandbox.rs`：包装 `crates/corpus/src/drivers/sandbox_driver/mod.rs` 已有的 namespace/seccomp/cgroup 原语，实现 `SandboxHost`，按 probe 选可用组合（namespace + seccomp + Landlock/cgroup），返回 `IsolationStrength` + 缺失保证（源 §5.1 + §8.2）。
- [ ] Secret 存储：桌面用户会话（Secret Service）与 headless 服务采用不同 backend，token 不明文入配置（源 §5.1）→ `credential.rs` 首版接口。

**Files:** `crates/platform-linux/src/sandbox.rs`, `crates/platform-linux/src/credential.rs`

### H1-7 上层接入（Workspace Tools + Pi Runtime + Dasein 走 Host capability）

- [ ] Workspace Tools：`crates/corpus/src/tools/tools/{bash_exec,process_list,apply_patch,file_read,file_write,file_search,grep}.rs` 的直接 `tokio::process` / `tokio::fs` 改为经 `platform-host` 的 `ProcessHost`/`FilesystemHost`（与 coding 线 Wave 2 Workspace Tools V2 协同；旧工具名保留 alias）。
- [ ] Pi Runtime：Pi adapter 的进程 spawn / worktree 文件操作统一走 Host capability（coding 线 Wave 3 依赖此契约）。
- [ ] Dasein：`crates/dasein/src/impl/security/rollback/mod.rs:459-661`（btrfs/systemctl/cp/stat 直调）改为经 `ServiceHost`/`ProcessHost`（这是总纲 §5 Wave 2「Dasein/Executive 命令走 Host Capability」的落点，**依赖 H1-2/H1-5**）。
- [ ] `exec-server`：`crates/exec-server/Cargo.toml:15` 依赖收窄——改依赖 `platform-api`/`platform-host` sandbox 契约而非整个 `corpus`（与 arch A2 协同）。

**Files:** `crates/corpus/src/tools/tools/*.rs`, `crates/dasein/src/impl/security/rollback/mod.rs`, `crates/exec-server/Cargo.toml`

### H1-8 Contract test suite（Linux 分支，源 §9.1）

- [ ] `crates/platform-linux/tests/contract_*.rs`：argv/空格/Unicode/大输出/非 UTF-8/环境变量；父进程退出→孤儿子进程回收、超时、强杀、无残留；symlink 逃逸拒绝；原子写冲突/expected hash/磁盘满/权限拒绝；watcher 重复/乱序/溢出;service 不存在/权限不足/启动超时;sandbox 能力探测与降级报告。
- [ ] 这套 contract test 设计为 backend 无关（H2/H3 复用同一套断言）。

**Files:** `crates/platform-linux/tests/contract_process.rs`, `contract_fs.rs`, `contract_service.rs`, `contract_sandbox.rs`

**H1 验收（源 §10 H1 / §9.3）：** Linux CLI Coding Agent 全闭环**只通过 Host API 执行**（无残留 `Command::new` 旁路）；进程树在取消/崩溃/超时后无残留；workspace confinement 有逃逸测试；无 silent fallback（降级必上报 manifest）；systemd D-Bus 真实生效、无 systemd 有显式降级。测试需完整 Rust env：`bash scripts/cargo-agent.sh test -p platform-linux`（本机 Linux 可跑）；systemd VM 路径走 nightly CI。

---

## H2 — Windows Core（deferred，押到 coding 线 Wave 5 之后）

> 轻量占位——启动前另开独立 spec。仅列骨架与验收，不展开文件级。

- [ ] 建 `crates/platform-windows/`：`ProcessHost` 用 `CreateProcessW`（原生 UTF-16 argv/env）；每 operation 一个 Job Object（CPU/mem/子进程限制 + kill-on-close 保证清理，源 §6.1）。
- [ ] ConPTY 交互终端（输入输出分别排空，避免同步管道死锁）。
- [ ] `ServiceHost` 直调 Service Control Manager API，不解析 `sc.exe` 本地化文本。
- [ ] Filesystem watcher `ReadDirectoryChangesW`；IPC 命名管道 + ACL；凭据 Windows Credential Manager。
- [ ] Sandbox 首版 = 受限 token + Job + ACL；AppContainer 作为需单独验证的强化 backend。
- [ ] 路径测试覆盖 drive letter/UNC/长路径/大小写/junction/reparse/reserved names（源 §6.2）。
- [ ] 复用 H1-8 backend 无关 contract test suite。

**H2 验收（源 §10 H2）：** Windows 上完成"搜索→编辑→测试→取消→清理"且**无残留进程**；跑通与 Linux 同一套 Pi/Native Runtime E2E。CI：`windows-latest` + real ConPTY/Job contract tests（源 §9.2）。

---

## H3 — macOS Core（deferred，随 H2 之后）

> 轻量占位——启动前另开独立 spec。

- [ ] 建 `crates/platform-macos/`：`ProcessHost` 用 `posix_spawn` + process group（子进程树 + 超时回收语义与 Linux 一致）。
- [ ] `ServiceHost` 面向用户 Agent 与系统 Daemon 分别生成/管理 launchd plist（源 §7.1）。
- [ ] 文件监听 FSEvents + 单文件精确监听的目录快照/去重层；凭据 Keychain；IPC Unix domain socket/XPC。
- [ ] Sandbox 以最小权限 + 文件授权 + 签名 entitlement 建可审计模型（不用命令行技巧）。
- [ ] 打包：code signing + notarization + universal binary + 升级回滚；TCC 权限状态映射（Screen Recording/Accessibility/Automation/Microphone 分别建模，未授予返回 `PermissionRequired` 而非伪装失败，源 §7.2）。
- [ ] 复用 H1-8 contract test suite。

**H3 验收（源 §10 H3）：** Intel/Apple Silicon 支持矩阵清晰；核心 Agent E2E 通过。CI：`macos-latest` + Intel/Apple Silicon 打包验证（源 §9.2）。

---

## H4 — Desktop capability（deferred，GUI 不阻塞核心 Runtime）

> 轻量占位——启动前另开独立 spec。将现有 Linux 桌面驱动重组进 `platform-linux/src/desktop/`。

- [ ] Linux：把 `crates/corpus/src/drivers/{display/x11,display/window_x11,input/uinput,a11y/atspi}.rs` 重组为 `platform-linux/src/desktop/{x11,wayland,atspi}/`、`.../input/uinput/`、`.../display/framebuffer/`（源 §5.2）。X11 与 Wayland 能力在 manifest 分开报告；uinput 需显式���备权限（不静默提权）；AT-SPI 优先于像素坐标点击。
- [ ] Windows：UI Automation 为首选结构化 backend；SendInput/屏幕捕获/剪贴板各自独立授权（源 §6.2）。
- [ ] macOS：Accessibility API 结构化控制；Screen Recording/Accessibility 独立 TCC 建模（源 §7.2）。
- [ ] 统一 `observe` 与 `input` 权限，**默认禁用输入注入**；Headless CI 用虚拟显示测试，但不据此宣称真实 Wayland compositor 已验证（源 §5.2）。

**H4 验收（源 §10 H4）：** 每平台至少一个结构化 Accessibility backend；像素后备路径有显式降级标记。CI 增桌面 session test。

---

## H5 — 生产运营（deferred，随三平台成熟度推进）

> 轻量占位——启动前另开独立 spec。

- [ ] 安装 / 升级 / 回滚 / 崩溃恢复自动化（三平台，源 §10 H5 + §9.3）。
- [ ] 平台遥测与兼容性报告（capability manifest 上报，标记 Core/Desktop/Sandbox 各自成熟度）。
- [ ] Nightly 真机矩阵 + 长期稳定性测试（Linux systemd VM / Windows real ConPTY / macOS Intel+Apple Silicon，源 §9.2）。
- [ ] 权限模型审计闭环：所有 capability grant（`host.service.manage:aletheon` / `host.fs.write:/workspace/project` 等，源 §8.1）可解释、可撤销、可审计。

**H5 验收（源 §9.3 发布门槛）：** 平台 backend 无 silent fallback；Agent 进程树在取消/崩溃/超时后无残留；workspace confinement 有跨平台逃逸测试；权限请求可解释/可撤销/可审计；安装/升级/回滚/卸载均有自动化验证；生产支持矩阵明确标记 Core/Desktop/Sandbox 成熟度。

---

## 明确不做（源 §12）

- 不继续把所有 OS 能力塞进单个 `PlatformAdapter`（用小 trait + capability）。
- 不通过解析 shell 命令输出模拟原生服务 API（systemd 走 D-Bus，SCM 走 API）。
- 不把 Android stub 计为成熟平台（`crates/corpus/src/drivers/platform/android.rs` 保持 stub，不进生产矩阵）。
- 不因 Windows/macOS 未完成而冻结 Linux Agent 生产化（H1 独立推进）。
- 不把 ROS/CAN/GPIO/机器人关节塞入 Host Platform（硬边界，源 §1.2）。
- 不提供无法说明底层保证的统一 `sandbox=true` 布尔值（必返回 `IsolationStrength` + 缺失保证）。
- 不先建十几个 `*-api` crate 再找调用者——H0 只建 api/host/linux 三个，且 H0-6 立即接 facade。

---

## 测试与环境备注

- **统一入口**：所有构建/测试 `bash scripts/cargo-agent.sh test -p <crate> <filter>`，禁止裸 cargo。
- **本机可跑**：`platform-api`（类型/序列化）、`platform-linux`（Linux contract test，非 systemd 部分）。
- **需 per-OS CI runner**：systemd D-Bus（Linux systemd VM nightly）、Windows ConPTY/Job/SCM、macOS launchd/FSEvents/签名公证、桌面 session test。
- **CI 必须区分**（源 §9.2）：编译 / 单元测试 / OS contract test / 桌面 session test / 安装升级 test —— 不能以 `cargo check --all-targets` 冒充平台可用性验证。
