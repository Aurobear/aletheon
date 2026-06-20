# Aletheon 架构重设计

**日期**：2026-06-21  
**状态**：设计完成，待实现  
**作者**：Aurobear + Claude  

---

## 1. 背景与动机

### 1.1 当前问题

1. **crate 命名冗余**：所有 crate 带 `aletheon-` 前缀（`aletheon-abi`, `aletheon-body`, `aletheon-brain` 等）
2. **body 是"上帝 crate"**：~105 个文件，包含工具、沙盒、安全、MCP、感知、驱动、TUI、技能、钩子、ACIX
3. **安全概念重复**：`aletheon-body` 和 `aletheon-self` 都有完整的安全管线（loop_detector, circuit_breaker, output_guardrail 等）
4. **runtime 过大**：~110 个文件，混杂核心编排和应用功能
5. **内存系统分裂**：`aletheon-memory` 和 `aletheon-runtime/src/impl/memory/` 职责重叠
6. **brain 编译时耦合硬件**：依赖 body 的 `features = ["input", "display", "a11y", "ocr"]`
7. **通信层抽象不足**：`aletheon-comm` 的 IPC 管理器是硬编码的，没有统一传输抽象
8. **通用模块未解耦**：错误处理、日志、配置、同步原语分散在各 crate 中

### 1.2 设计目标

1. **命名简洁**：去掉 `aletheon-` 前缀，选择更精确的名字
2. **职责清晰**：每个 crate 有单一明确的职责
3. **依赖简洁**：消除循环依赖，减少编译耦合
4. **通信统一**：参考 Linux 内核设计统一的通信架构
5. **通用模块解耦**：参考 Linux `include/` 将公共接口集中到基础层
6. **渐进迁移**：分 4 阶段实施，每阶段独立可验证

---

## 2. 新 Crate 结构

### 2.1 命名映射

| 旧名 | 新名 | 职责 |
|---|---|---|
| `aletheon-abi` | `base` | 接口定义 + 通信协议 + 通用模块（错误、日志、配置、同步） |
| `aletheon-body` | `corpus` | 核心执行体（精简后：沙盒、MCP、感知、平台适配） |
| `aletheon-brain` | `cognit` | 认知计算引擎（推理、规划、反思、学习） |
| `aletheon-comm` | `comm` | 总线路由层（消息路由、传输实现、子系统注册） |
| `aletheon-memory` | `memory` | 记忆系统（SQLite 后端、向量存储、压缩） |
| `aletheon-self` | `dasein` | 自我层（SelfField 策略引擎、DaseinModule） |
| `aletheon-runtime` | `runtime` | 内核调度（ReActLoop、会话管理、编排） |
| `aletheon-meta` | `metacog` | 自演化引擎（形态发生、基因组、元认知） |
| （从 body 拆出） | `drivers` | 硬件驱动（display/input/ocr/a11y） |
| （从 body 拆出） | `tools` | 工具实现（20+ 内置工具） |
| （从 body 拆出） | `security` | 安全管线（策略引擎、循环检测、断路器、输出护栏） |
| （从 body 拆出） | `interact` | 交互层（CLI、TUI、ACIX） |
| `aletheond` | `daemon` | 守护进程入口 |
| `aletheon-cli` | `cli` | CLI/TUI 客户端 |
| `aletheon-exec` | `exec` | 独立执行二进制 |

### 2.2 目录结构

```
aletheon/
├── crates/
│   ├── base/               ← 原 aletheon-abi，扩展为完整"头文件层"
│   │   ├── src/
│   │   │   ├── subsystem/  ← 子系统 trait 定义
│   │   │   ├── types/      ← 共享类型
│   │   │   ├── protocol/   ← 通信协议定义
│   │   │   ├── error/      ← 错误类型、Result、错误码
│   │   │   ├── log/        ← 日志宏、追踪接口
│   │   │   ├── config/     ← 配置解析、验证、热加载
│   │   │   ├── sync/       ← 锁原语、异步原语、通道
│   │   │   └── dasein/     ← DaseinModule ABI 类型
│   │
│   ├── comm/               ← 收窄为总线路由层
│   │   ├── src/
│   │   │   ├── router/     ← 消息路由
│   │   │   ├── transport/  ← 传输实现（Unix socket、共享内存、io_uring）
│   │   │   ├── registry/   ← 子系统注册/发现/生命周期
│   │   │   └── bus/        ← 事件总线实现
│   │
│   ├── corpus/             ← 精简后的核心执行体
│   │   ├── src/
│   │   │   ├── core/       ← 执行核心（沙盒、MCP、感知、平台适配）
│   │   │   └── bridge/     ← 对外接口
│   │
│   ├── drivers/            ← 新 crate，硬件驱动
│   │   ├── src/
│   │   │   ├── display/    ← X11, DRM, 剪贴板
│   │   │   ├── input/      ← uinput
│   │   │   ├── ocr/        ← tesseract
│   │   │   └── a11y/       ← AT-SPI
│   │
│   ├── tools/              ← 新 crate，工具实现
│   │   ├── src/
│   │   │   ├── builtin/    ← 内置工具（bash, file, grep, glob, etc.）
│   │   │   ├── search/     ← 工具搜索（BM25+TF-IDF）
│   │   │   ├── output/     ← 输出管理（capture, pruner, truncation）
│   │   │   └── registry/   ← 工具注册表
│   │
│   ├── security/           ← 新 crate，安全管线
│   │   ├── src/
│   │   │   ├── policy/     ← 策略引擎
│   │   │   ├── detector/   ← 循环检测、风险分类
│   │   │   ├── breaker/    ← 断路器
│   │   │   ├── guardrail/  ← 输出护栏
│   │   │   └── audit/      ← 审计
│   │
│   ├── interact/           ← 新 crate，交互层
│   │   ├── src/
│   │   │   ├── cli/        ← CLI 接口
│   │   │   ├── tui/        ← TUI（ratatui）
│   │   │   └── acix/       ← ACIX
│   │
│   ├── cognit/             ← 保持完整，内部重组
│   ├── memory/             ← 保持
│   ├── dasein/             ← 原 aletheon-self
│   ├── runtime/            ← 保持
│   └── metacog/            ← 原 aletheon-meta
│
├── binaries/
│   ├── daemon/             ← 原 aletheond
│   ├── cli/                ← 原 aletheon-cli
│   └── exec/               ← 原 aletheon-exec
```

---

## 3. 依赖关系图

```
base  (无内部依赖——叶子 crate)
    ^
    |
    +--- comm           (base)
    +--- memory          (base)
    +--- security        (base)
    +--- tools           (base)
    +--- drivers         (base)
    +--- interact        (base)
    |
    +--- corpus          (base, comm, memory, security, tools)
    |       ^
    |       |
    +--- cognit          (base, comm, corpus[features: drivers,interact])
    |       ^
    |       |
    +--- dasein          (base, corpus, cognit, comm, memory)
    |       ^
    |       |
    +--- runtime         (base, cognit, corpus, comm, memory, dasein, metacog)
    |
    +--- metacog         (base)

daemon     (runtime)
cli        (interact[features: drivers])
exec       (base, runtime, corpus, cognit)
```

### 关键改进

- **base 是唯一叶子**：所有 crate 只依赖 base，不互相依赖
- **comm 不再被 corpus/cognit 直接依赖**：通过 base 的抽象接口通信
- **security 独立**：corpus 和 dasein 共用同一个 security crate
- **drivers 独立**：cognit 不再编译时依赖硬件驱动，通过 feature flag 按需启用
- **tools 独立**：corpus 只调用 tools 的接口，不关心具体工具实现

---

## 4. 通信架构（参考 Linux 内核）

### 4.1 消息模型（类似 Netlink）

```rust
// base/protocol/message.rs
pub struct Envelope {
    pub source: SubsystemId,      // 发送方
    pub target: SubsystemId,      // 接收方
    pub kind: MessageKind,        // 消息类型
    pub payload: Payload,         // 消息体
    pub correlation_id: u64,      // 请求-响应关联
    pub priority: Priority,       // 优先级
    pub timestamp: Instant,       // 时间戳
}

pub enum MessageKind {
    Request,      // 请求
    Response,     // 响应
    Event,        // 事件（广播）
    Signal,       // 信号（异步通知）
}

pub enum Payload {
    Bytes(Vec<u8>),           // 原始字节
    Json(serde_json::Value),  // JSON
    Typed(Box<dyn Any>),      // 类型化（通过 base 定义的类型）
}
```

### 4.2 子系统注册（类似设备模型）

```rust
// base/subsystem/mod.rs
pub trait Subsystem: Send + Sync {
    fn id(&self) -> SubsystemId;
    fn name(&self) -> &str;
    fn capabilities(&self) -> &[Capability];
    fn handle_message(&self, msg: Envelope) -> Result<Response>;
    fn lifecycle(&self) -> SubsystemLifecycle;
}

pub enum SubsystemLifecycle {
    Initializing,
    Running,
    Suspended,
    ShuttingDown,
    Failed(String),
}
```

### 4.3 传输抽象（类似 VFS）

```rust
// base/transport/mod.rs
pub trait Transport: Send + Sync {
    fn send(&self, msg: Envelope) -> Result<()>;
    fn recv(&self) -> Result<Envelope>;
    fn try_recv(&self) -> Result<Option<Envelope>>;
    fn close(&self) -> Result<()>;
}

// comm/transport/ 实现
pub struct UnixSocketTransport { ... }
pub struct SharedMemoryTransport { ... }
pub struct IoUringTransport { ... }
pub struct InProcessTransport { ... }  // 同进程内的 channel
```

### 4.4 系统调用表（类似 syscall table）

```rust
// base/syscall/mod.rs
pub struct SyscallTable {
    handlers: HashMap<SyscallId, Box<dyn SyscallHandler>>,
}

pub trait SyscallHandler: Send + Sync {
    fn call(&self, args: &[Value]) -> Result<Value>;
}
```

---

## 5. 数据流

### 5.1 请求处理流（用户输入 → 响应）

```
用户输入
    ↓
cli/interact (TUI/CLI 接收)
    ↓ Envelope { source: cli, target: runtime, kind: Request }
comm/router (路由到 runtime)
    ↓
runtime (调度)
    ↓ Envelope { source: runtime, target: cognit, kind: Request }
cognit (推理)
    ↓ Envelope { source: cognit, target: runtime, kind: Response }
    ↓ (可能触发工具调用)
runtime (分发工具调用)
    ↓ Envelope { source: runtime, target: corpus, kind: Request }
corpus (执行)
    ↓ Envelope { source: corpus, target: tools, kind: Request }
tools (具体工具执行)
    ↓ Envelope { source: tools, target: corpus, kind: Response }
corpus → runtime → cognit → runtime → cli → 用户
```

### 5.2 安全检查流

```
每个 Envelope 经过 comm/router 时：
    ↓
security/policy (策略检查)
    ↓ 允许/拒绝/需要审批
security/detector (循环检测)
    ↓ 正常/可疑/危险
security/guardrail (输出过滤)
    ↓ 通过/过滤
继续路由
```

### 5.3 子系统生命周期

```
daemon 启动
    ↓
comm/registry 注册所有子系统
    ↓
每个子系统 SubsystemLifecycle::Initializing
    ↓
comm/registry 检查所有子系统就绪
    ↓
所有子系统 SubsystemLifecycle::Running
    ↓
正常运行...
    ↓
daemon 关闭信号
    ↓
comm/registry 通知所有子系统 SubsystemLifecycle::ShuttingDown
    ↓
每个子系统优雅关闭
    ↓
comm/registry 确认所有子系统关闭
    ↓
进程退出
```

---

## 6. 错误处理

### 6.1 统一错误类型（在 base 中定义）

```rust
// base/error/mod.rs
#[derive(Debug, thiserror::Error)]
pub enum AletheonError {
    #[error("subsystem {subsystem} error: {message}")]
    Subsystem { subsystem: SubsystemId, message: String },

    #[error("communication error: {0}")]
    Communication(#[from] CommError),

    #[error("security violation: {reason}")]
    Security { reason: String, severity: Severity },

    #[error("timeout after {duration:?}")]
    Timeout { duration: Duration },

    #[error("resource exhausted: {resource}")]
    ResourceExhausted { resource: String },

    #[error("internal error: {0}")]
    Internal(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Low,
    Medium,
    High,
    Critical,
}

pub type Result<T> = std::result::Result<T, AletheonError>;
```

### 6.2 错误传播路径

```
tools 执行失败
    ↓ AletheonError::Subsystem
corpus 捕获
    ↓ 包装为 Envelope { kind: Response, payload: Error }
runtime 捕获
    ↓ 决策：重试/回退/报告
cognit 反思
    ↓ 记录到 memory
用户看到友好错误信息
```

### 6.3 错误码体系（类似 Linux errno）

```rust
// base/error/codes.rs
pub mod codes {
    pub const SUCCESS: i32 = 0;
    pub const EPERM: i32 = 1;      // 权限不足
    pub const ENOENT: i32 = 2;     // 资源不存在
    pub const EINTR: i32 = 4;      // 中断
    pub const EIO: i32 = 5;        // I/O 错误
    pub const EAGAIN: i32 = 11;    // 重试
    pub const ENOMEM: i32 = 12;    // 内存不足
    pub const EACCES: i32 = 13;    // 访问拒绝
    pub const EBUSY: i32 = 16;     // 资源忙
    pub const ETIMEDOUT: i32 = 110; // 超时
}
```

---

## 7. 测试策略

### 7.1 分层测试

| 层级 | 测试类型 | 覆盖范围 |
|---|---|---|
| **base** | 单元测试 | 错误类型序列化、协议编解码、配置解析 |
| **comm** | 集成测试 | 消息路由、传输层、子系统注册 |
| **tools** | 单元测试 | 每个工具独立测试，mock 依赖 |
| **security** | 单元测试 + 属性测试 | 策略引擎、循环检测、断路器状态机 |
| **corpus** | 集成测试 | 沙盒执行、MCP 客户端、感知源 |
| **cognit** | 集成测试 | 推理链、规划、反思（mock LLM） |
| **dasein** | 单元测试 | 8 层策略引擎、DaseinModule |
| **runtime** | 端到端测试 | ReActLoop、会话管理、编排 |
| **metacog** | 集成测试 | 形态发生流水线、基因组加载 |

### 7.2 Mock 基础设施

```rust
// base/testing/mock_transport.rs
pub struct MockTransport {
    sent: Vec<Envelope>,
    recv_queue: Vec<Envelope>,
}

impl Transport for MockTransport {
    fn send(&self, msg: Envelope) -> Result<()> { ... }
    fn recv(&self) -> Result<Envelope> { ... }
}

// 每个子系统都有 Mock 实现
// base/testing/mock_subsystem.rs
pub struct MockSubsystem { ... }
impl Subsystem for MockSubsystem { ... }
```

---

## 8. 迁移计划（渐进式，4 阶段）

### 阶段 1：重命名 crate（1 周）

**目标**：只改名字，不改结构

| 步骤 | 操作 | 验证 |
|---|---|---|
| 1.1 | 重命名目录：`aletheon-abi` → `base` 等 | `cargo check` 通过 |
| 1.2 | 更新 Cargo.toml 的 `name` 字段 | `cargo check` 通过 |
| 1.3 | 更新所有 `use aletheon_xxx::` 为 `use xxx::` | `cargo check` 通过 |
| 1.4 | 更新 binaries 的依赖引用 | `cargo check` 通过 |
| 1.5 | 更新文档中的 crate 名称 | 文档一致性 |

**风险**：低——纯重命名，不影响功能
**验证**：`cargo test --workspace` 全部通过

### 阶段 2：重构 base（2 周）

**目标**：将 base 扩展为完整的"头文件层"

| 步骤 | 操作 | 验证 |
|---|---|---|
| 2.1 | 从 comm 移入协议定义（Envelope, Protocol, Transport） | `cargo check` 通过 |
| 2.2 | 从各 crate 移入通用错误类型 | `cargo check` 通过 |
| 2.3 | 从各 crate 移入配置系统 | `cargo check` 通过 |
| 2.4 | 从各 crate 移入可观测性接口 | `cargo check` 通过 |
| 2.5 | 从各 crate 移入同步原语 | `cargo check` 通过 |
| 2.6 | 添加子系统 trait 定义 | `cargo check` 通过 |

**风险**：中——涉及大量代码移动
**验证**：`cargo test --workspace` 全部通过

### 阶段 3：拆分 corpus（2 周）

**目标**：将 corpus（原 body）拆分为多个独立 crate

| 步骤 | 操作 | 验证 |
|---|---|---|
| 3.1 | 创建 `drivers` crate，移入硬件驱动 | `cargo check` 通过 |
| 3.2 | 创建 `tools` crate，移入工具实现 | `cargo check` 通过 |
| 3.3 | 创建 `security` crate，移入安全管线 | `cargo check` 通过 |
| 3.4 | 创建 `interact` crate，移入 CLI/ACIX/TUI | `cargo check` 通过 |
| 3.5 | 精简 `corpus` 为核心执行体 | `cargo check` 通过 |
| 3.6 | 更新所有依赖引用 | `cargo check` 通过 |

**风险**：高——最大的结构变化
**验证**：`cargo test --workspace` 全部通过

### 阶段 4：重设计 comm（1 周）

**目标**：将 comm 收窄为总线路由层

| 步骤 | 操作 | 验证 |
|---|---|---|
| 4.1 | 实现消息路由器 | 单元测试通过 |
| 4.2 | 实现子系统注册表 | 单元测试通过 |
| 4.3 | 实现传输抽象层 | 单元测试通过 |
| 4.4 | 迁移现有 EventBus 到新架构 | 集成测试通过 |
| 4.5 | 更新所有子系统使用新通信接口 | `cargo test --workspace` 通过 |

**风险**：中——涉及通信层重写
**验证**：端到端测试通过

---

## 9. 时间线

```
Week 1:     阶段 1（重命名）
Week 2-3:   阶段 2（重构 base）
Week 4-5:   阶段 3（拆分 corpus）
Week 6:     阶段 4（重设计 comm）
Week 7:     集成测试 + 文档更新
```

---

## 10. 风险与缓解

| 风险 | 影响 | 缓解措施 |
|---|---|---|
| 重命名导致编译失败 | 低 | 分步重命名，每步验证 |
| base 过大 | 中 | 使用 feature flag 控制模块启用 |
| corpus 拆分破坏功能 | 高 | 保持接口不变，只移动代码 |
| comm 重写引入 bug | 中 | 先写测试，再实现 |
| 循环依赖 | 高 | 严格遵循依赖图，不允许反向依赖 |

---

## 11. 设计决策记录

### 11.1 内部结构模式

**决策**：各 crate 自组织内部结构，不强制统一模式。

**理由**：
- 不同 crate 有不同的职责和复杂度，强制统一模式会导致不必要的抽象
- `base` 是纯定义层，适合扁平结构
- `comm` 是实现层，适合按功能分目录
- `corpus` 是执行体，可能需要 `core/bridge` 分层
- `cognit` 是认知引擎，可能需要 `pipeline/stage` 分层
- Linux 内核也没有强制的内部结构模式

**影响**：
- 每个 crate 可以根据自己的职责选择最自然的组织方式
- 唯一的共同点是：都用 `base` 的接口定义、一致的错误处理、一致的编码风格

### 11.2 DaseinModule 哲学命名

**决策**：保留 DaseinModule 的哲学命名（sorge, bewandtnis, negativity 等）。

**理由**：
- 这些命名反映了 `dasein` crate 的哲学基础（存在主义）
- 已有代码和文档中广泛使用这些术语
- 对于理解系统的设计哲学有帮助

**影响**：
- 新贡献者需要学习这些术语
- 文档中需要提供术语表

### 11.3 Feature Flag 管理

**决策**：通过拆分 crate 减少 feature flag 臃肿。

**理由**：
- 当前 `aletheon-body` 有 8 个 feature flag，导致复杂的条件编译路径
- 拆分后，`drivers` crate 可以独立管理硬件相关的 feature flag
- `tools` crate 不需要硬件 feature flag
- `cognit` 通过 `corpus[features: drivers,interact]` 按需启用

**影响**：
- 编译时间可能减少（不需要编译不需要的硬件驱动）
- 依赖图更清晰
- 每个 crate 的 feature flag 更少、更专注

---

## 12. 成功标准

1. **所有 crate 命名符合新规范**
2. **`cargo test --workspace` 全部通过**
3. **依赖图无循环**
4. **每个 crate 职责清晰，文件数 < 50**
5. **通信层统一，支持多种传输后端**
6. **通用模块集中在 base 中**
7. **编译时间不显著增加**
