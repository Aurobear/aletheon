# Phase 4：Communication Fabric V2

## 1. 目标

通信体系只统一协议语义，不强迫所有交互经过一个 Bus。

| 类型 | 语义 | 机制 |
|---|---|---|
| Call/Query | 同进程立即返回、维护状态不变量 | trait call |
| Command | 改变状态、可排队、返回 receipt | command queue |
| Event | 已发生事实 | append + publish |
| Mailbox Message | Agent Process 协作 | request/response/signal |
| Stream | token、日志、telemetry | bounded channel |

## 2. Envelope V2

位置：`crates/fabric/src/contract/envelope_v2.rs`，迁移期间不要直接破坏旧 Envelope。

```rust
pub struct EnvelopeV2 {
    pub id: MessageId,
    pub schema: SchemaId,
    pub source: Endpoint,
    pub target: Target,
    pub pattern: DeliveryPattern,

    pub operation_id: Option<OperationId>,
    pub causation_id: Option<MessageId>,
    pub correlation_id: Option<MessageId>,
    pub namespace: NamespaceId,

    pub logical_time: LogicalTime,
    pub deadline: Option<MonoDeadline>,
    pub priority: Priority,
    pub payload: Payload,
}
```

## 3. Mailbox

每个 Process 一个逻辑 Mailbox：

```rust
#[async_trait]
pub trait MailboxService {
    async fn send(&self, target: ProcessId, msg: EnvelopeV2) -> Result<DeliveryReceipt>;
    async fn request(&self, target: ProcessId, msg: EnvelopeV2) -> Result<EnvelopeV2>;
    async fn signal(&self, target: ProcessId, signal: ProcessSignal) -> Result<()>;
    async fn recv(&self, mailbox: MailboxId) -> Result<EnvelopeV2>;
}
```

第一阶段 backend 使用 Tokio bounded channel。

## 4. Stream

所有 Stream 必须声明：

```text
capacity
overflow policy
cancellation
end reason
```

可选 overflow：

```text
BlockProducer
DropOldest
DropNewest
FailStream
```

LLM token 默认 `BlockProducer`；机器人 telemetry 默认 `DropOldest`。

## 5. CommunicationBus 定位

保留 `CommunicationBus` 作为路由与 Transport 适配器：

```text
Module/Process target resolution
in-process fast path
Unix socket transport
request correlation
topic routing
```

它不负责：

- Process lifecycle；
- Permission；
- 业务事务；
- Agora commit；
- 替代 Cognit/Memory/Dasein trait call。

## 6. 旧系统迁移

### PR-4A

- 新增 EnvelopeV2 与转换器；
- 不修改旧 EventBus（已完成；旧 EventBus trait 现已删除）。

### PR-4B

- Process mailbox 使用 EnvelopeV2；
- SubAgent 协作切到 mailbox。

### PR-4C

- daemon JSON-RPC 请求映射为 OperationCommand；
- streaming event 映射为 TurnEvent stream。

### PR-4D

- 新代码禁止实现旧 `Event` trait（已完成：旧 Event trait 已移除，CommunicationBus 为单一系统）；
- 旧 EventBus 通过 LegacyEventBridge 兼容（迁移已完成；LegacyEventBridge 仅在过渡期使用）；

### PR-4E

- 旧 Event/EventBus 使用点已清零，旧接口已删除（迁移完成）。

## 7. Schema 与兼容

每个跨进程 payload 必须有稳定 schema：

```text
aletheon.turn.request/v1
aletheon.turn.event/v1
aletheon.process.signal/v1
aletheon.capability.request/v1
aletheon.capability.result/v1
```

未知 schema 返回结构化 `UnsupportedSchema`，不能静默按 JSON 猜测。

## 8. 测试

```bash
cargo test -p fabric envelope_v2
cargo test -p fabric mailbox
cargo test -p fabric stream_backpressure
cargo test -p executive process_messaging
```

必须覆盖：

- request/response correlation；
- deadline 到期；
- mailbox 满时行为；
- receiver 退出；
- signal 优先于普通消息；
- Legacy Event 转换；
- schema 版本拒绝。

## 9. 完成标准

- Agent 协作不再依赖直接函数调用；
- 新业务代码不使用旧 Event trait；
- CommunicationBus 不持有业务服务；
- daemon、in-process 和未来 remote transport 共享 EnvelopeV2。

