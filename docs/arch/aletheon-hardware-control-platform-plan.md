# Aletheon Hardware Control Platform 生产化计划

> 文档版本：1.0
>
> 更新日期：2026-07-19
>
> 范围：设备发现、传感、执行、机器人、ROS 2、串口、CAN、GPIO 与安全控制
>
> 边界：Aletheon 负责高层意图、能力治理与可验证操作，不承担硬实时闭环

---

## 0. 结论

Aletheon 当前没有统一的硬件控制平台。相关概念分散在 Kernel capability、Fabric `BodyRuntime`、Corpus drivers、Dasein 感知设计和机器人架构文档中，但缺少一个能够实际生产使用的 Device/Robot control plane。

`BodyRuntime::Action { name, parameters: JSON }` 可以作为远程 Runtime 的通用信封，却不能直接充当硬件控制 API：它没有稳定设备身份、类型化命令、遥测 schema、控制租约、时钟域、deadline、序列号、安全状态、校准、急停和故障恢复语义。

应建立独立的 **Hardware Control Platform**：

- Aletheon 负责设备对象、权限、租约、高层命令、工作流、证据和恢复决策。
- Hardware Broker 负责 provider 发现、连接、路由、审计和流量治理。
- Robot/Device Edge Runtime 负责本地实时控制、驱动、watchdog 和最终安全裁决。
- 任何云端/Agent 命令都不能绕过本地 Safety Supervisor。

第一阶段先做模拟器和只读遥测，再做 ROS 2 仿真，然后才做真实执行器。

---

## 1. 与 Host Platform 的边界

### 1.1 Hardware Control 负责

- 设备稳定身份、发现、清单、健康、固件和校准状态。
- 传感器数据、事件、遥测 schema、采样率和时间戳。
- 执行器命令、范围限制、deadline、确认和停止。
- 控制租约、共享/独占访问、deadman 和 watchdog。
- ROS 2、CAN、串口、GPIO/I2C/SPI 等 provider。
- 机器人 namespace、模式、任务状态和 Safety Supervisor 集成。
- 仿真、SIL、HIL、lab、production 的环境隔离。
- 高频数据边缘缓存、降采样、摘要和 Artifact 归档。

### 1.2 Hardware Control 不负责

- 通用文件、进程、Shell、PTY 和主机服务管理。
- Windows/macOS/Linux 的安装、升级和桌面用户会话。
- 键盘、鼠标、窗口、剪贴板和普通桌面屏幕控制。
- 模型自主执行 1 kHz 电机控制循环。
- 用 Agora 作为高频时序数据库或原始传感器总线。

Hardware Provider 可以依赖 Host Platform 获取串口、socket、进程或文件访问，但不能把 provider 协议泄漏给 Executive 和 Cognit。

---

## 2. 当前代码审计

### 2.1 已有可复用部分

- Kernel 已有 Capability 与命名空间概念，可表达类似 `robot.command:/robot/lab/kuavo-01` 的授权。
- Fabric `BodyRuntime` 已有 Runtime 边界和 action/result 传输思路。
- 宏内核架构文档已经明确 Aletheon 与 Robot Runtime 的实时边界。
- Dasein 感知设计已有 `device_list/device_configure` 方向。
- Agora、Artifact、审计与 verifier 可承接任务级状态和证据。

### 2.2 生产缺口

1. 没有 `DeviceId`、`DeviceManifest`、`DeviceClass` 等稳定领域类型。
2. 没有 discovery registry 和 provider 生命周期。
3. 命令是任意字符串 + JSON，无法静态限制危险动作。
4. 没有 exclusive lease、租约续期、断连失效和所有权转移。
5. 没有 command sequence、monotonic deadline、幂等键和确认等级。
6. 没有 telemetry QoS、背压、丢帧策略、时钟同步和 schema 演进。
7. 没有统一 health/fault/calibration/firmware 模型。
8. 没有急停与 Safety Supervisor 的不可绕过规则。
9. 没有 ROS 2、serial、SocketCAN、GPIO 的真实依赖和 provider。
10. 没有 fault injection、SIL/HIL 和真机发布门禁。

因此，现阶段不能通过给 `BodyRuntime` 增加几个 action name 来宣称已经支持硬件控制。

---

## 3. 控制平面与实时数据平面

```text
用户 / Executive / Cognit
          │  WorkOrder + policy + acceptance criteria
          ▼
Kernel Capability Broker
          │  grant + lease scope
          ▼
Hardware Broker ───────────────► Audit / Agora / Artifact
          │ typed command             summaries/events/logs
          ▼
Provider: ROS2 / CAN / Serial / GPIO / Vendor SDK
          │
          ▼
Robot or Device Edge Runtime
          │
          ├── Safety Supervisor
          ├── real-time controller
          ├── driver / bus
          └── local telemetry buffer
```

关键原则：

- Aletheon 发“导航到安全点”“抓取目标”“切换检查模式”等高层命令。
- Edge Runtime 将高层命令分解为轨迹与实时控制。
- Safety Supervisor 有权拒绝、裁剪、暂停或终止命令。
- 网络断开、租约过期或 deadline 到期时，设备进入预定义 fail-safe，而不是继续最后命令。
- 原始 1 kHz 遥测保留在边缘缓冲或专用时序存储；Agora 只接收摘要、关键帧和事件。

---

## 4. crate 与模块划分

```text
crates/hardware-api       # 领域类型、trait、schema、错误、状态机
crates/hardware-broker    # registry、provider、lease、policy、routing
crates/hardware-sim       # deterministic simulator 与 fault injection
crates/hardware-ros2      # ROS 2 graph/topic/service/action/lifecycle adapter
crates/hardware-serial    # 串口 transport 与 framing，不含具体设备业务
crates/hardware-can       # SocketCAN/CAN-FD/ISO-TP provider
crates/hardware-gpio      # Linux GPIO chardev v2 adapter
crates/hardware-vendor-*  # 厂商 SDK 或具体机器人 provider
```

依赖方向：

```text
Executive/Cognit -> Kernel capability -> hardware-api
hardware-broker -> hardware-api + selected providers
provider -> hardware-api + platform-api
hardware-api -X-> ROS/serial/CAN/vendor crates
```

`hardware-api` 必须保持传输无关，不能出现 ROS message、串口路径或厂商句柄。

---

## 5. 设备对象模型

### 5.1 核心标识

```rust
pub struct DeviceId(Uuid);

pub struct DeviceUri {
    pub namespace: DeviceNamespace,
    pub provider: ProviderId,
    pub path: Vec<String>,
}

pub enum DeviceNamespace {
    Simulation,
    Lab,
    Production,
}

pub enum DeviceClass {
    Robot,
    Actuator,
    Sensor,
    Camera,
    Audio,
    ComputeAccelerator,
    Bus,
    Composite,
}
```

不要以 `/dev/ttyUSB0`、`can0`、ROS node name 作为永久身份。这些是 provider endpoint，会随重启、拓扑和部署变化。稳定身份应由序列号、证书、公钥、厂商 ID、部署清单或显式绑定产生。

### 5.2 DeviceManifest

```rust
pub struct DeviceManifest {
    pub id: DeviceId,
    pub uri: DeviceUri,
    pub class: DeviceClass,
    pub model: String,
    pub firmware: Option<String>,
    pub capabilities: Vec<DeviceCapability>,
    pub command_schemas: Vec<CommandSchemaRef>,
    pub telemetry_schemas: Vec<TelemetrySchemaRef>,
    pub safety_profile: SafetyProfileRef,
    pub calibration: CalibrationState,
    pub trust: DeviceTrust,
}
```

manifest 需要签名/来源和版本，Broker 发现的运行时事实不能覆盖管理员批准的安全上限。

### 5.3 状态模型

```text
Unknown -> Discovered -> Identified -> Ready
                          │            │
                          ▼            ▼
                       Untrusted     Degraded
                                       │
                                       ▼
                                     Faulted
                                       │ reset + checks
                                       ▼
                                     Ready
```

执行器额外有 `Safe/Armed/Active/Stopping/EStopped`。`EStopped` 只能通过本地规定流程恢复，Agent 不应拥有通用“清除急停”权限。

---

## 6. Provider 与 Broker API

### 6.1 Provider 接口

```rust
pub trait HardwareProvider: Send + Sync {
    async fn probe(&self) -> Result<ProviderManifest, HardwareError>;
    async fn discover(&self, query: DeviceQuery) -> Result<Vec<DeviceManifest>, HardwareError>;
    async fn observe(&self, device: DeviceId, request: ObserveRequest)
        -> Result<TelemetryStream, HardwareError>;
    async fn execute(&self, lease: LeaseToken, command: TypedCommand)
        -> Result<CommandReceipt, HardwareError>;
    async fn stop(&self, lease: LeaseToken, request: StopRequest)
        -> Result<StopReceipt, HardwareError>;
}
```

### 6.2 Broker 职责

- Provider 注册、健康检查、重连和版本兼容。
- Endpoint 到 stable `DeviceId` 的绑定。
- Capability/policy 检查和租约仲裁。
- Typed command schema 验证、单位转换和上限检查。
- sequence、deadline、幂等与 replay 防护。
- telemetry subscription、QoS、背压与 Artifact 分流。
- operation receipt、审计、trace 和关键安全事件上报。
- namespace 隔离，禁止 sim 配置误发 production。

Broker 不应把 ROS、CAN 等 provider 特有 API 重新包装成无穷多个特殊方法；它应稳定设备生命周期与治理语义，业务能力通过版本化 schema 扩展。

---

## 7. 命令、租约与安全

### 7.1 TypedCommand

```rust
pub struct TypedCommand {
    pub command_id: CommandId,
    pub device: DeviceId,
    pub schema: CommandSchemaId,
    pub payload: ValidatedPayload,
    pub sequence: u64,
    pub issued_at: MonotonicInstant,
    pub deadline: MonotonicInstant,
    pub idempotency_key: Option<IdempotencyKey>,
    pub requested_ack: AckLevel,
}
```

命令 schema 包含单位、范围、速率限制、前置状态、互斥规则和安全类别。模型输出的 JSON 必须先解析并验证为 `TypedCommand`，不能直接进入 provider。

### 7.2 控制租约

```rust
pub struct ControlLease {
    pub lease_id: LeaseId,
    pub holder: ActorId,
    pub device: DeviceId,
    pub capabilities: CapabilitySet,
    pub mode: LeaseMode,
    pub expires_at: MonotonicInstant,
    pub deadman: DeadmanPolicy,
}
```

规则：

- 读遥测通常可共享；执行器控制默认独占。
- Lease 有短 TTL，需要心跳续约。
- Broker 和 Edge Runtime 都验证 lease，云端判断不能取代边缘判断。
- 断连、进程崩溃、holder 被取消或续约失败会触发 fail-safe。
- 高风险设备需要人类在场、双重批准或物理钥匙等额外约束。
- 权限 scope 精确到 namespace、device、capability 和 command class。

### 7.3 停止层级

```text
Cancel Task    停止高层任务，不保证设备已静止
ControlledStop 请求 Edge Runtime 按安全轨迹停车
SafeHold       保持安全姿态/制动状态
EmergencyStop  本地硬件/安全系统最高优先级
```

Agent UI 和日志必须明确区分四者，不能把“任务取消成功”等同于“设备已经安全停止”。

---

## 8. 遥测、时钟与数据治理

### 8.1 TelemetryEnvelope

```rust
pub struct TelemetryEnvelope {
    pub device: DeviceId,
    pub stream: StreamId,
    pub schema: TelemetrySchemaId,
    pub sequence: u64,
    pub source_time: DeviceTime,
    pub receive_time: MonotonicInstant,
    pub quality: DataQuality,
    pub payload: BytesOrArtifactRef,
}
```

必须保留 source time 与 receive time；不同设备时钟域不能假装相同。时间同步状态作为 telemetry metadata 输出。

### 8.2 流量分层

```text
Level 0: Edge raw buffer        高频、短期、设备本地
Level 1: Operational stream     降采样、背压、诊断使用
Level 2: Agora state/events     任务状态、告警、摘要、关键帧
Level 3: Artifact archive       rosbag/MCAP/CAN trace/video/故障包
```

订阅必须声明可靠性、最大延迟、队列长度、丢弃策略和采样率。传感器洪峰不能拖垮 Executive、Agora 或主 Agent event loop。

---

## 9. Provider 计划

### 9.1 Simulator

模拟器不是 demo，而是整个安全模型的可执行规范：

- 固定种子的 deterministic clock。
- 设备发现、租约、命令确认和 telemetry。
- 断连、延迟、乱序、重复、传感器冻结、校准失效。
- stuck actuator、limit violation、lease expiry、E-stop。
- 可重放 operation receipt 和事件轨迹。

### 9.2 ROS 2

- Graph discovery 映射到 provider endpoint，而非直接作为稳定 `DeviceId`。
- Topic 用于 telemetry，Service 用于短请求，Action 用于有反馈和取消的长任务。
- Managed node lifecycle 映射为 provider health/ready 状态。
- QoS 显式配置，不依赖默认值。
- SROS 2/DDS security 身份与 Aletheon capability 需要桥接，但不能互相替代。
- 大体积 rosbag/MCAP 进入 Artifact，不进入 Agora 消息正文。

ROS 2 provider 应参考官方的 [Managed nodes 生命周期设计](https://design.ros2.org/articles/node_lifecycle.html)；具体 ROS 发行版的 QoS 与安全配置在实现时锁定版本并纳入兼容性矩阵。

### 9.3 Serial

- Host 只提供端口和权限；Hardware provider 负责 baud、framing、CRC、握手和设备身份。
- 热插拔后重新识别设备，不能按端口号自动恢复写权限。
- 协议 parser 采用有界缓冲，防止异常长度和无终止帧。
- 写命令需要 ack/timeout/retry policy；非幂等命令默认不自动重试。

### 9.4 CAN / CAN-FD

- Linux 首版使用 SocketCAN，后续由其他 OS/provider 扩展。
- 原始 frame provider 与设备协议 provider 分层。
- 支持 filter、error frame、bus-off、restart、CAN-FD 和可选 ISO-TP。
- 发送权限按 interface + CAN ID/range + frame type 限制。
- bus-off 和错误帧进入 health/fault，不仅写日志。

Linux 官方文档将 SocketCAN 作为基于网络栈的 CAN socket 接口，并支持队列、过滤及错误消息：[SocketCAN](https://docs.kernel.org/networking/can.html)。

### 9.5 GPIO / I2C / SPI

- GPIO 使用 character device API v2，不使用旧 sysfs GPIO。
- 默认只读发现；输出 line 需要独占 lease 和 safe default。
- 优先使用成熟内核子系统驱动，不用用户态 bit-banging 替代已有驱动。
- I2C/SPI provider 必须绑定批准的 bus/address/device schema，禁止模型任意扫描生产总线。

Linux GPIO v2 以 chip 和 line request 为核心，同时提供 edge event、sequence 和 timestamp；内核也明确建议优先使用已有专用驱动：[GPIO Character Device Userspace API](https://docs.kernel.org/userspace-api/gpio/chardev.html)。

---

## 10. 与 BodyRuntime、Kernel、Agora 的集成

### 10.1 BodyRuntime

保留 `BodyRuntime` 作为通用 Runtime transport，但新增类型化 hardware envelope：

```text
BodyRuntime Action
  kind = hardware.command.v1
  payload = serialized TypedCommand
  lease = scoped LeaseToken
  deadline = monotonic-relative duration + broker timestamp
```

Provider 不能仅凭 `Action.name` 执行；必须经 Hardware Broker 解码、schema 验证和安全检查。

### 10.2 Kernel Capability

示例：

```text
hardware.observe:/robot/lab/kuavo-01/camera/front
hardware.command:/robot/lab/kuavo-01/navigation
hardware.stop:/robot/lab/kuavo-01
hardware.calibrate:/device/lab/imu-02
hardware.admin:/bus/lab/can0
```

`hardware.command` 不隐含 `hardware.admin` 或 `emergency-stop.reset`。

### 10.3 Agora 与 Artifact

Agora 保存：目标、当前模式、租约 holder、任务状态、摘要、告警、关键 receipt。Artifact 保存：原始日志、rosbag/MCAP、CAN trace、视频、校准包和故障快照。二者都不在实时安全链路中。

---

## 11. 验证体系

### 11.1 测试金字塔

```text
Schema/property tests
Provider contract tests
Deterministic simulator + fault injection
SIL with real protocol stack
HIL with non-dangerous fixture
Lab robot supervised trials
Production canary
```

### 11.2 必测故障

- Lease 过期、续约乱序、重复 holder。
- stale command、deadline 过期、sequence 回退、重复发送。
- Broker 崩溃、Provider 崩溃、网络分区、设备重启。
- telemetry flood、队列溢出、数据乱序和时钟跳变。
- 传感器冻结、NaN/越界、校准过期。
- 执行器无响应、部分完成、stop ack 丢失。
- E-stop 触发、恢复条件不足、错误复位请求。
- sim/lab/production namespace 混淆。
- 错误设备绑定和热插拔身份漂移。

### 11.3 生产门槛

- 模型无法直接产生未经 schema 与 policy 验证的总线写操作。
- 租约过期和 Broker 断连有可证明的 fail-safe。
- 本地 Safety Supervisor 不依赖 Aletheon 在线。
- 每条命令可追踪到 actor、capability grant、lease、schema 和 receipt。
- 高风险路径有 HIL 和人工监督记录。
- raw telemetry 不会压垮 Agent 控制平面。
- 设备、provider、固件和 schema 兼容性矩阵可查询。

---

## 12. 分阶段 PR 计划

### D0：Hardware API 与模拟器

- 新建 `hardware-api`、`hardware-broker`、`hardware-sim`。
- 定义 Device、Manifest、Provider、Telemetry、TypedCommand、Lease、Fault。
- 实现 deterministic simulator 和 fault injection。
- Kernel capability 接入，默认只有 observe 权限。

验收：可在无真实硬件情况下验证租约过期、断连和安全停止。

### D1：Linux 只读发现与遥测

- 发现串口、CAN interface、GPIO chip、摄像头和计算加速器。
- 建立 stable binding 流程和 Device Registry。
- 只开放 health/telemetry，不开放执行器写入。
- 完成 Artifact 分流和背压测试。

验收：热插拔不造成设备身份误绑定；高频流不进入 Agora 正文。

### D2：ROS 2 仿真

- ROS 2 provider、graph discovery、topic/service/action/lifecycle。
- 接 Gazebo 或现有机器人仿真环境。
- 实现高层导航/模式命令、反馈、取消和超时。
- 引入 QoS 与安全身份配置。

验收：在仿真中完成“获取租约→执行→反馈→取消/完成→释放”。

### D3：Serial 与 CAN 实设备

- Serial framing/CRC/ack provider。
- SocketCAN/CAN-FD/error frame/provider health。
- 使用非危险测试夹具和 loopback，默认不接电机。
- 完成 bus 权限、限速、日志和故障注入。

验收：异常帧、bus-off、拔插和非幂等重试行为均可预测。

### D4：受控执行器

- Typed actuator commands、硬限制、deadman、watchdog。
- Exclusive lease、controlled stop、safe hold。
- 双层 Broker + Edge lease 验证。
- 人工批准与现场安全流程。

验收：断网、进程崩溃和 lease 过期均进入预期安全状态。

### D5：机器人 HIL 与 Lab

- 选择一个目标机器人，不同时铺开多个厂商。
- 对接本地 Safety Supervisor、模式管理和状态摘要。
- HIL、lab namespace、操作员 checklist、故障恢复演练。
- 生成完整 evidence bundle。

验收：在监督环境连续运行并通过规定故障场景，才可进入 production canary。

### D6：多设备与远程站点

- Device federation、边缘 broker、离线策略和证书轮换。
- 带宽分层、断连缓存、时间同步质量。
- 跨站点 capability 和租约不得隐式继承。

---

## 13. 推荐的最小纵向切片

不要先实现所有总线。推荐第一个端到端切片：

```text
Hardware Simulator
  -> 一个虚拟移动机器人
  -> pose/battery/health telemetry
  -> acquire/renew/release lease
  -> typed NavigateTo + ControlledStop
  -> deadline + sequence + receipt
  -> disconnect/lease-expiry fault injection
  -> Agora summary + Artifact event log
```

第二个切片才连接 ROS 2 仿真；第三个切片连接只读 CAN/Serial 测试夹具。这样可以先证明控制模型，再引入真实总线复杂度。

---

## 14. 明确不做

- 不让 LLM 直接写 `/dev/tty*`、`can0` 或 GPIO line。
- 不把任意 JSON action 当作类型安全硬件命令。
- 不让云端 Agent 承担硬实时或最终安全职责。
- 不在第一版同时接入多个机器人厂商。
- 不以“ROS topic 能收到消息”作为生产完成标准。
- 不将取消任务、停止动作和急停混为一谈。
- 不让模拟、实验室和生产 namespace 共享默认写权限。

Hardware Control 的成功标准，是 Aletheon 能在失联、崩溃、延迟、重复命令和设备故障下仍保持可预测、可审计、可停止，而不是简单地“能让硬件动起来”。
