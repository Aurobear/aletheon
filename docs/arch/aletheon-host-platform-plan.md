# Aletheon Host Platform 多操作系统生产化计划

> 文档版本：1.0
>
> 更新日期：2026-07-19
>
> 目标平台：Linux、Windows、macOS
>
> 边界：只处理 Aletheon 在宿主操作系统上的运行、隔离与桌面交互，不负责机器人和工业硬件控制

---

## 0. 结论

Aletheon 目前还不能称为跨平台 Runtime。现有 `PlatformAdapter` 位于 Corpus，能力集中在服务管理、主机信息和提权；实际实现只有 Linux/Android，且 Linux 路径强依赖 `systemctl`、`/proc` 和 Linux 桌面栈。文档宣称的 process/fs/net/device 统一层与代码并不一致，Windows、macOS 也没有 adapter。

应单独建立 **Host Platform**：为 Kernel、Runtime、Workspace Tools 和桌面自动化提供稳定的宿主能力接口。上层只依赖能力与语义，不依赖 `systemctl`、Win32、launchd、X11 等平台细节。

推荐顺序：

1. 先冻结平台无关契约并完成 Linux 生产实现。
2. 再实现 Windows，优先保证 CLI Agent、进程树、PTY、文件和服务能力。
3. 再实现 macOS，并处理 launchd、权限授权、Accessibility 和签名公证。
4. 最后扩大桌面自动化覆盖，不让 GUI 能力阻塞核心 Agent Runtime。

---

## 1. 与 Hardware Control Platform 的硬边界

### 1.1 Host Platform 负责

- 操作系统和主机信息。
- 进程创建、进程树、信号/终止、资源限制和输出收集。
- Shell、PTY、终端尺寸、编码与环境变量。
- 文件读写、原子替换、元数据、路径规范化、文件监听。
- TCP/UDP/Unix socket/Named Pipe 等宿主通信原语。
- 服务安装、启动、停止、重启和状态查询。
- Sandbox、凭据、权限提升和用户会话。
- 键盘、鼠标、窗口、剪贴板、屏幕、通知、Accessibility。
- 宿主摄像头、麦克风、显示器等普通媒体外设。
- 安装包、自动升级、日志、崩溃转储和操作系统生命周期集成。

### 1.2 Host Platform 不负责

- 机器人关节、电机、机械臂、底盘和执行器。
- CAN、GPIO、I2C、SPI 等设备控制语义。
- ROS 2 graph、topic、service、action 和 lifecycle。
- 硬实时控制循环、状态估计、轨迹控制和本地 Safety Supervisor。
- 设备控制租约、急停、校准、故障复位和高频遥测。

摄像头和麦克风按归属区分：宿主会议设备属于 Host Media；机器人载荷属于 Hardware Provider。串口枚举可由 Host 暴露原始端点，但协议、设备身份和控制权归 Hardware。

---

## 2. 当前代码审计

### 2.1 已有实现

当前相关代码主要位于：

```text
crates/corpus/src/drivers/platform/
  adapter.rs
  linux.rs
  android.rs
  mod.rs

crates/corpus/src/drivers/
  display/
  input/
  a11y/
  proc/
  io/
  sandbox/
```

现有 `PlatformAdapter` 主要提供：

- `name/is_available/capabilities`
- `list/status/start/stop/restart service`
- `hostname/kernel_version/uptime`
- `is_root/elevate_privileges`

### 2.2 关键问题

1. **层级错误**：Host 原语放在 Corpus；Corpus 应提供 Agent 可理解的语义工具，而非承载 OS HAL。
2. **契约虚高**：设计文档描述了 process/fs/net/device，trait 实际没有这些能力。
3. **Linux 实现过窄**：依赖 `systemctl`、`/proc`，不能覆盖无 systemd 容器、WSL、桌面用户会话或最小发行版。
4. **伪抽象**：声称 D-Bus 的实现仍调用命令，连接对象没有成为真实控制通道。
5. **平台缺口**：Windows、macOS adapter 不存在；Android 当前为 stub，不应计入生产覆盖。
6. **直接调用泛滥**：Workspace、Runtime、Corpus 各自可能调用 `tokio::process`、`tokio::fs`、`walkdir` 或 shell，无法集中治理。
7. **桌面能力混杂**：X11、uinput、AT-SPI 等 Linux 专用驱动没有归入清晰的 Linux Desktop backend。
8. **缺少可观测性**：没有统一 operation id、结构化错误、资源用量、审计收据和能力探测快照。

---

## 3. 目标架构

```text
Executive / Corpus / Capability Runtime / Workspace Tools
                         │
                         ▼
                Kernel Capability Broker
                         │
                         ▼
                   platform-api
                         │
            ┌────────────┼────────────┐
            ▼            ▼            ▼
     platform-linux platform-windows platform-macos
            │            │            │
       Linux APIs      Win32 APIs   Darwin/Cocoa APIs
```

建议 crate：

```text
crates/platform-api       # 稳定类型、trait、错误与 capability descriptor
crates/platform-host      # backend 选择、策略桥接、operation receipts
crates/platform-linux     # Linux 服务、进程、sandbox、desktop
crates/platform-windows   # Win32 服务、Job、ConPTY、desktop
crates/platform-macos     # launchd、FSEvents、Keychain、Accessibility
```

Corpus 中现有 platform 代码先保留 compatibility facade，内部改为调用 `platform-host`；迁移完成后再标记 deprecated，避免一次性破坏所有调用者。

---

## 4. 稳定 API 设计

### 4.1 不使用一个巨型 PlatformAdapter

使用小 trait 和显式 capability：

```rust
pub trait HostInfo: Send + Sync {
    async fn snapshot(&self) -> Result<HostSnapshot, HostError>;
}

pub trait ProcessHost: Send + Sync {
    async fn spawn(&self, spec: SpawnSpec) -> Result<ProcessHandle, HostError>;
    async fn inspect(&self, id: ProcessId) -> Result<ProcessSnapshot, HostError>;
    async fn signal(&self, id: ProcessId, signal: ProcessSignal) -> Result<(), HostError>;
    async fn terminate_tree(&self, id: ProcessId, grace: Duration) -> Result<ExitStatus, HostError>;
}

pub trait FilesystemHost: Send + Sync {
    async fn metadata(&self, path: HostPath) -> Result<EntryMetadata, HostError>;
    async fn atomic_write(&self, request: AtomicWrite) -> Result<WriteReceipt, HostError>;
    async fn watch(&self, request: WatchRequest) -> Result<EventStream<FsEvent>, HostError>;
}

pub trait PtyHost: Send + Sync {}
pub trait NetworkHost: Send + Sync {}
pub trait ServiceHost: Send + Sync {}
pub trait SandboxHost: Send + Sync {}
pub trait CredentialHost: Send + Sync {}
pub trait UserSessionHost: Send + Sync {}
pub trait DesktopHost: Send + Sync {}
pub trait MediaHost: Send + Sync {}
```

### 4.2 必须统一的语义

- 路径使用逻辑 `HostPath`，同时保存 native path；不能把 Windows 路径硬转成 Unix 字符串。
- 进程参数使用 `argv`，不允许默认拼 shell command string。
- 文本输出显式携带 encoding、截断状态和 Artifact 引用。
- 进程终止以“进程树”为单位；超时先 graceful，再强制终止。
- 文件写入支持 expected hash、临时文件、fsync policy 和原子替换结果。
- 每次敏感操作携带 `OperationContext`：actor、workspace、capability、policy decision、deadline、trace id。
- 所有错误映射为稳定 `HostErrorKind`，同时保留平台 native code 和诊断细节。
- API 返回 operation receipt，不能只返回 `bool`。

### 4.3 Capability manifest

```rust
pub struct HostCapabilityManifest {
    pub platform: HostPlatform,
    pub os_version: String,
    pub arch: Architecture,
    pub backend_version: Version,
    pub features: BTreeMap<HostFeature, FeatureState>,
    pub constraints: Vec<HostConstraint>,
    pub probed_at: SystemTime,
}
```

`FeatureState` 至少区分：`Available`、`Unavailable`、`PermissionRequired`、`Degraded`、`Unsupported`。编译成功不等于能力可用，启动时必须 runtime probe。

---

## 5. Linux 实现计划

### 5.1 核心 Runtime

- `ProcessHost`：process group、pidfd（可用时）、退出状态、stdout/stderr backpressure。
- 资源限制：优先 cgroup v2；不可用时降级到 rlimit，并明确报告降级。
- `PtyHost`：Unix PTY，处理窗口尺寸、信号和 UTF-8/原始字节流。
- `FilesystemHost`：openat 风格的目录相对访问，避免字符串路径检查后再打开造成 TOCTOU。
- 文件监听：inotify 为普通 backend；需要系统级安全审计时另设高权限 backend，不混入默认 Agent 安装。
- `ServiceHost`：systemd D-Bus backend；无 systemd 时返回 unsupported 或使用显式配置的 supervisor backend。
- `SandboxHost`：namespace、seccomp、Landlock/cgroup；容器环境通过探测选择可用组合。
- Secret 存储：桌面用户会话与 headless 服务采用不同 backend，不把 token 明文放入配置文件。

Linux 资源隔离应对齐内核的 [cgroup v2 文档](https://docs.kernel.org/admin-guide/cgroup-v2.html)，但 Host API 只暴露平台无关的 CPU、内存、进程数和 I/O policy。

### 5.2 Linux Desktop

将现有驱动重组为：

```text
platform-linux/src/desktop/x11/
platform-linux/src/desktop/wayland/
platform-linux/src/desktop/atspi/
platform-linux/src/input/uinput/
platform-linux/src/display/framebuffer/
```

注意事项：

- X11 截屏/注入不代表 Wayland 能力；manifest 必须分开报告。
- uinput 需要显式设备权限，不能靠静默提权。
- AT-SPI 属于 Accessibility backend，应优先于像素坐标点击。
- Headless CI 必须有虚拟显示测试，但不得据此宣称真实 Wayland compositor 已验证。

---

## 6. Windows 实现计划

### 6.1 核心 Runtime

- `ProcessHost` 使用 `CreateProcessW`，参数和环境使用原生 UTF-16。
- 每个 Agent operation 创建 Job Object，限制 CPU、内存和子进程，并使用 kill-on-close 保证清理。
- 交互式终端使用 ConPTY，输入和输出分别排空，避免同步管道死锁。
- `ServiceHost` 直接调用 Service Control Manager API，不解析 `sc.exe` 本地化文本。
- 文件监听用 `ReadDirectoryChangesW`；大规模增量索引可以后续增加 USN Journal backend。
- IPC 默认命名管道，并使用 ACL 限制当前用户/服务身份。
- 凭据使用 Windows Credential Manager 或受保护的服务凭据方案。
- Sandbox 首版使用受限 token、Job 和 ACL 组合；AppContainer 作为需要单独兼容验证的强化 backend。

微软官方说明 Job Object 可将进程组作为单元实施限制、终止和资源核算，适合作为 Agent 进程树边界：[Job Objects](https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects)。交互式 shell 使用微软的 [ConPTY](https://learn.microsoft.com/en-us/windows/console/creating-a-pseudoconsole-session)，服务管理对接 [Service Control Manager](https://learn.microsoft.com/en-us/windows/win32/services/service-control-manager)，强化隔离再评估 [AppContainer](https://learn.microsoft.com/en-us/windows/win32/secauthz/appcontainer-isolation)。

### 6.2 Windows Desktop

- UI Automation 为首选结构化交互 backend。
- SendInput、屏幕捕获、剪贴板作为独立 capability，分别授权。
- Service Session 0 与登录用户桌面会话必须分开；后台服务不能假设可直接控制桌面。
- UAC 提权是显式用户流程，不实现通用的 `elevate_privileges()` 黑盒。
- 路径测试覆盖 drive letter、UNC、长路径、大小写、junction、reparse point 和 reserved names。

---

## 7. macOS 实现计划

### 7.1 核心 Runtime

- `ProcessHost` 使用 `posix_spawn`/process group，确保子进程树和超时回收语义一致。
- `ServiceHost` 面向用户 Agent 与系统 Daemon 分开生成和管理 launchd plist。
- 文件监听使用 FSEvents；单文件精确监听需要补充目录快照与去重层。
- 凭据使用 Keychain。
- IPC 使用 Unix domain socket/XPC adapter，选择由部署形态决定。
- Sandbox 不以已经不适合作为公共生产接口的命令行技巧为基础；先以最小权限、文件授权和签名 entitlement 建立可审计模型。
- 打包必须覆盖 code signing、notarization、universal binary 和升级回滚。

Apple 对后台服务推荐采用 launchd，并区分系统 daemon 与登录用户 agent：[Creating Launch Daemons and Agents](https://developer.apple.com/library/archive/documentation/MacOSX/Conceptual/BPSystemStartup/Chapters/CreatingLaunchdJobs.html)。文件事件使用 [FSEvents](https://developer.apple.com/documentation/coreservices/file_system_events)，需要系统安全事件时才评估具备更高权限和审批要求的 [Endpoint Security](https://developer.apple.com/documentation/endpointsecurity)。

### 7.2 macOS Desktop

- Accessibility API 负责结构化 UI 控制。
- Screen Recording、Accessibility、Automation、Microphone 等 TCC 权限分别建模。
- 权限未授予应返回 `PermissionRequired` 和引导信息，不能伪装成工具执行失败。
- GUI Agent 运行在登录用户 session；系统 daemon 只承担 broker/服务职责。

---

## 8. 安全与权限模型

### 8.1 禁止通用提权入口

删除上层对 `elevate_privileges()` 的依赖，改为具体、可解释的 capability grant：

```text
host.service.manage:aletheon
host.process.spawn:workspace
host.fs.write:/workspace/project
host.desktop.observe
host.desktop.input
host.media.camera
host.credential.read:provider/openai
```

权限请求必须包含用途、范围、持续时间、actor 和回收策略。

### 8.2 Sandbox 分层

```text
L0 Observe     只读信息与无副作用探测
L1 Workspace   仅 workspace 文件和受控进程
L2 Networked   在 L1 基础上开放经过 policy 的网络
L3 Desktop     允许用户桌面观察/输入
L4 Admin       极少数服务安装或系统配置操作
```

不同 OS 的底层保证不同，API 必须返回 `IsolationStrength` 和缺失保证；不得把 Windows Job、Linux namespace 和 macOS TCC 描述为完全等价。

---

## 9. 测试与生产门槛

### 9.1 Contract test suite

每个 backend 必须通过相同测试：

- argv/空格/Unicode/大输出/非 UTF-8/环境变量。
- 父进程退出、孤儿子进程、超时、强杀和清理。
- symlink/junction/reparse point 逃逸。
- 原子写冲突、expected hash、磁盘满和权限拒绝。
- 文件监听重复、乱序、溢出与重建索引。
- 服务不存在、权限不足、启动超时和崩溃循环。
- Sandbox 能力探测与降级报告。
- 用户退出、睡眠/唤醒、网络变化和系统重启。

### 9.2 CI 矩阵

```text
Linux:  ubuntu-latest + rootless container + systemd VM nightly
Windows: windows-latest + real ConPTY/Job contract tests
macOS:   macos-latest + Intel/Apple Silicon packaging validation
```

CI 必须区分：编译、单元测试、OS contract test、桌面 session test、安装升级 test。不能以 `cargo check --all-targets` 代替平台可用性验证。

### 9.3 发布门槛

- 平台 backend 无 silent fallback。
- Agent 进程树在取消、崩溃和超时后无残留。
- workspace confinement 有跨平台逃逸测试。
- 所有权限请求可解释、可撤销、可审计。
- 安装、升级、回滚、卸载均有自动化验证。
- 生产支持矩阵明确标记 Core、Desktop、Sandbox 各自成熟度。

---

## 10. 分阶段 PR 计划

### H0：冻结边界与契约

- 新建 `platform-api`、`platform-host`。
- 定义小 trait、错误、receipt、manifest、runtime probe。
- 为旧 `PlatformAdapter` 建 compatibility facade。
- 建立跨平台 compile CI。

验收：Corpus/Kernel 不再新增 OS 特判；旧 API 有迁移告警。

### H1：Linux 核心生产化

- 迁移 process/fs/pty/service/sandbox。
- 修复 systemd D-Bus 与无 systemd 降级。
- Workspace Tools 和 Pi Runtime 统一走 Host capability。
- 加入进程树、路径 confinement 与大输出测试。

验收：Linux CLI Coding Agent 全闭环只通过 Host API 执行。

### H2：Windows Core

- Job Object、ConPTY、Filesystem、SCM、Named Pipe。
- Windows 安装器、日志和升级骨架。
- 跑通同一套 Pi/Native Runtime E2E。

验收：Windows 上完成“搜索→编辑→测试→取消→清理”且无残留进程。

### H3：macOS Core

- Process、PTY、FSEvents、launchd、Keychain。
- 签名、公证、universal binary 和升级回滚。
- TCC 权限状态映射。

验收：Intel/Apple Silicon 支持矩阵清晰，核心 Agent E2E 通过。

### H4：Desktop capability

- Linux X11/Wayland/AT-SPI 重组。
- Windows UI Automation。
- macOS Accessibility/Screen Recording。
- 统一 observe 与 input 权限，默认禁用输入注入。

验收：每个平台至少有一个结构化 Accessibility backend；像素后备路径有显式降级标记。

### H5：生产运营

- 安装/升级/回滚/崩溃恢复。
- 平台遥测和兼容性报告。
- Nightly 真机矩阵与长期稳定性测试。

---

## 11. 建议的近期任务顺序

```text
第 1 周：platform-api + compatibility facade + contract tests
第 2 周：Linux ProcessHost/PtyHost + operation receipts
第 3 周：FilesystemHost + confinement + watcher
第 4 周：Linux Service/Sandbox + Pi Runtime 接入
第 5-6 周：Windows Job/ConPTY/Filesystem/SCM
第 7-8 周：macOS Process/FSEvents/launchd/Keychain
后续：三平台安装升级、Desktop 能力和真机矩阵
```

这条路线与 Hardware Control 独立推进。Host H1 是 Agent 生产化的前置条件；Hardware 模拟器开发不必等待 Windows/macOS 完成。

---

## 12. 明确不做

- 不继续把所有 OS 能力塞进单个 `PlatformAdapter`。
- 不通过 shell 命令输出解析模拟原生服务 API。
- 不把 Android stub 计为成熟平台。
- 不因 Windows/macOS 尚未完成而冻结 Linux Agent 生产化。
- 不把 ROS、CAN、GPIO、机器人关节塞入 Host Platform。
- 不提供无法说明底层保证的统一 `sandbox=true` 布尔值。

最终成功标准不是“能在三个 OS 编译”，而是同一个受治理的 Agent operation 在 Linux、Windows、macOS 上具有一致的生命周期、权限、证据与失败语义。
