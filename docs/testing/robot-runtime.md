# Robot 运行时与仿真验收基线

> 架构边界：[runtime authority ADR 的 Robot contracts and safety boundaries](../decisions/adr-runtime-authority-and-contract-governance.md#robot-contracts-and-safety-boundaries)
> 历史基线：`origin/dev@c080b08bf3170dd8a09acdb738a255133abbf11b`（2026-08-05）。
> 安装态验收：2026-08-07，R8/X13 MuJoCo 链路 `passed`，实现由 PR #182 合入。
> 范围边界：**不包含物理实机验证**；HIL/real 独立 hard-stop/watchdog 证据仍是单独 gate。

## 1. 外部依赖（本仓库不复制源码）

公开仓库只保存版本化事实和占位符，不保存公司内网用户名、IP、绝对路径、隧道命令或凭据。实际访问参数由受控 operator config 提供。

| 能力 | 位置/入口 | 当前版本事实 |
|---|---|---|
| Kuavo bridge | `<robot-host>/<bridge-workspace>`；loopback gRPC endpoint 由部署配置解析 | 本地独立仓库（未配置 `origin`）；package `aletheon-kuavo-bridge` `0.1.0`；commit `1245538771b80f1b742d50f32041b801fba7b104`；tree `b403c574c1abb40772b8651831031eb369804010`；采样时工作树干净 |
| Bridge protocol（R0/远端旧运行态） | 远端旧 Bridge proto | SHA-256 `4a205a75ac7643d7769fbd7bd52f32faba64b4cdf9d81f6908617da490a7d7ff`；这是历史/负向 preflight 事实，不是当前 candidate digest |
| Bridge protocol（R8 accepted） | Aletheon proto 与 accepted bridge 使用同一 canonical proto | 两份文件逐字节相同；SHA-256 `77e44869ba6a6b3345753f699f9acd403a6c8f4de3383dcd5827b0ba7673e679`；已用于安装态 MuJoCo 验收 |
| ROS runtime | `<robot-host>/<ros-workspace>` | 早期只读采样时本机无 `/opt/ros`；后续安装态验收已通过受控远端 ROS/MuJoCo 链路，见 §2.9 |
| MuJoCo scene | `<robot-host>/<sim-workspace>` | sim commit `5fc79a6c48d5d7296763663d71fa52e0ff54d624`；`kuavo_assets` `10.3.0`；scene ID `kuavo-mujoco/default-v40`；`scene.xml` SHA-256 `052b37031446d5bc142eaf9d264711adc5bd57defeb1bc5c0a178fc747e4b25c`；asset tree `5881dc74c7ae3b828dfdb5b94f570f3ede9ccea8`；63 文件排序 manifest SHA-256 `5364678863725a47494cc9c734f20a02b864a5f7d16480bd1f57fe3cdd9e206d` |
| MuJoCo accepted scene | installed Robot version 53 runtime | scene ID `kuavo-mujoco/biped-s53`；正向、负向与三次连续真实 TUI 验收通过 |
| 设备标识 | 部署配置中的 `device_id` | example 值为 `kuavo-mujoco-01`；R1 将其纳入 typed resolved config，R8 再记录安装态实际值 |
| 连接方式 | operator config 管理的受控 tunnel/endpoint | 命令和地址不入库；evidence 只记录 endpoint identity digest |

R0 行保留 2026-08-05/06 的历史 provenance；当前 installed acceptance 以 §2.9 的同 SHA、official socket、真实 Policy/Bridge/ROS/MuJoCo 和 durable receipt 证据为准。

### 1.1 证据复现

```bash
# 在 Aletheon 、bridge 与 simulator 三个 checkout 中分别记录版本。
git rev-parse HEAD
git rev-parse 'HEAD^{tree}'

# 路径由 operator 配置解析，不把内部绝对路径入库。
BRIDGE_WORKSPACE=/path/to/bridge-workspace
SIM_WORKSPACE=/path/to/sim-workspace

# 校验 Aletheon/bridge proto。
sha256sum crates/hardware/proto/aletheon/embodiment/gateway/v1/gateway.proto \
  "$BRIDGE_WORKSPACE/proto/aletheon/embodiment/gateway/v1/gateway.proto"
cmp crates/hardware/proto/aletheon/embodiment/gateway/v1/gateway.proto \
  "$BRIDGE_WORKSPACE/proto/aletheon/embodiment/gateway/v1/gateway.proto"

# 生成 scene asset 的确定性 manifest digest。
cd "$SIM_WORKSPACE/src/kuavo_assets/models/biped_s40"
find . -type f -print0 | LC_ALL=C sort -z | xargs -0 sha256sum | sha256sum
```

## 2. 测试分层（禁止把 `#[ignore]` live test 的存在当成当次已通过）

| 层 | 入口 | 是否本轮验收 | 说明 |
|---|---|---|---|
| deterministic unit/contract | fabric/cognit/executive 定向测试 | 是 | 状态机、validator、verifier、episode sink、config |
| in-process robot session | `robot_session_e2e`、`robot_harness_composition` | 是 | Turn→EpisodeReport 链路，无外部依赖 |
| direct live bridge diagnostic | `robot_bridge_execute_e2e --ignored` | 仅 diagnostic | 需远程 bridge+MuJoCo；**不替代 R8 安装态验收** |
| installed full runtime | `/usr/bin/aletheon` official socket + 真实 Policy/bridge/MuJoCo | 是 | R8/X13 `accepted`；物理实机不是该门的前置，见 §2.9 |

### 2.1 2026-08-06 只读远端 preflight

本轮通过 operator 提供的受控 SSH 入口执行了只读检查。仓库不记录用户名、地址、隧道命令或
绝对路径。第一次 R2 摘要位于
`target/r2-readonly-preflight-20260806-074707/summary.json`，SHA-256
`5651708235a5e9959751ee48b68bf8dddd591cb6b1c35b9014966fe03ffb5480`；R7 关闭后的复核摘要位于
`target/r8-readonly-preflight-20260806-143924/summary.json`，SHA-256
`993a18ba65ed71dce6e8c34bcbceb438ed2ada909ed5ce35da57aa295a1c8fce`。

```text
local R2 provider --read-only tunnel--> remote loopback Bridge :50051
                                           |
                                           +--> ROS master :11311  connection refused
remote Policy :50052                        absent
remote installed Aletheon                   absent
```

运行 provenance：

| host | inspected checkout | running checkout | branch/HEAD | effective config | agree? |
|---|---|---|---|---|---|
| `<operator-robot-host>` | 本地主桥仓 base HEAD `1245538771b80f1b742d50f32041b801fba7b104` + 未部署的 R2/R6 candidate 工作树 | editable bridge source copy，package `0.1.0` | 远端副本无 `.git`；运行 proto SHA-256 `4a205a75...`，candidate 为 `77e44869...` | config SHA-256 `2987108a58d5427607b828a0354f98b34edc9efb180bbcba540a28768b45d811` | remote matches old base, not candidate；runtime not ready |
| `<operator-robot-host>` | kuavo ROS checkout `beta@442351d1c766fb598607583547eec4eccb3e0832` | 无 ROS master/MuJoCo 运行进程 | no running checkout | n/a | no runtime |
| `<operator-robot-host>` | Aletheon checkout为历史分支 | 无 installed/running Aletheon | no running checkout | n/a | no runtime |

只读 RPC 结果：

- Bridge proto SHA-256 仍为
  `4a205a75ac7643d7769fbd7bd52f32faba64b4cdf9d81f6908617da490a7d7ff`，与本地 candidate
  `77e44869ba6a6b3345753f699f9acd403a6c8f4de3383dcd5827b0ba7673e679` 不一致；
- `Health` 返回 `UNAVAILABLE`，ROS master/`rosnode`/`rostopic` 均不可达；
- `GetCapabilities` 返回 protocol `1.0`、provider `kuavo-noetic-mujoco` 和 device
  `kuavo-mujoco-01`，但 `max_message_bytes=0`、`max_progress_hz=0`；当前 Aletheon
  R2 provider 因 typed error `capability max_message_bytes must be nonzero` 正确 fail closed；
- `ListSkills` 返回两个 skill，但没有 input schema、preconditions 或 success criteria；配置
  manifest（SHA-256
  `3b0e18134283cdf334b85cabc0b3c8c6a68e97e747856a53a824ee0ead227049`）列出三个
  skill，说明 manifest 与运行 allowlist 尚未形成同一权威投影；
- `Snapshot` 返回 `base_pose/v1`、`base_twist/v1`、`ground_truth_pose/v1`，三者均 stale；
  运行实现还把 monotonic receive time 写入名为 `received_unix_ms` 的 wire 字段，不能作为
  Unix/跨机器 freshness evidence；
- Policy/ROS/MuJoCo 与 remote installed Aletheon 均不存在。复核只调用 `Health`、
  `GetCapabilities`、`ListSkills`、`Snapshot`；未调用 `ExecuteSkill`、`Cancel` 或 `SafeStop`，未修改
  远端文件，未重启任何服务。远端当时正在执行 operator 的无关 C++ build，因此没有运行测试或进行
  任何可能争用服务/构建资源的操作。

因此该结果是 R2/R8 的**历史真实负向 preflight**，不是安装态 acceptance。R2 的本地
capability/descriptor/schema/timestamp 契约当时已经 code-complete，但远端仍运行旧实现。后续 operator
授权的动作诊断见下一节；其后的 candidate 部署和完整安装态验收已完成，见 §2.9。

### 2.1.1 2026-08-06 operator 授权的 Version 53 MuJoCo 动作诊断

operator 授权后，以 `sudo` 启动 Robot version 53 ROS/MuJoCo 仿真并确认 `/real=false`。Bridge
重连后 Health 为 READY，`/odom` 与 `/ground_truth/state` 均持续产生 fresh sequence。受限
`kuavo.move_base_timed` 请求使用 `linear_x=0.05 m/s`、`duration_ms=1000`，operation ID
`5c9efdaf-456e-4639-847d-a54f89ef2322` 返回 `SKILL_OUTCOME_SUCCEEDED`，ground truth 平面位移
`0.02064 m`，terminal `SafeStop applied=true`。其后 3.261 秒内采集 32 个 strictly-increasing
观测，全部 fresh/READY；最大 XY speed `4.68e-6 m/s`、最大 |wz| `8.48e-6 rad/s`，stance 为
`1.0`。摘要位于 `target/r8-remote-sim-20260806-152812/summary.json`（SHA-256
`334a2b96a35ee6feda2b1d1ea9f785c3e14e34da32a56e72e7069912f6acdca5`），原始 bounded event
stream SHA-256 为 `d35b866248ab9d5aad56285ad91a2bb53f8d7938af8d27a6017346ed221f3083`。

这次真实诊断也发现 deployed old Bridge 的 ownership gate 是 **fail-open**：ROS master 实际返回
三个 `/cmd_vel` publisher，但旧实现把 XML-RPC `(code, message, value)` 外层误当作
`(publishers, subscribers, services)`，随后吞掉异常并返回空 publisher 集合。候选实现已改为正确
解析 envelope、排除 Bridge 自身并在 malformed/master failure 时拒绝动作
（`aletheon-kuavo-bridge/src/aletheon_kuavo_bridge/providers/kuavo_noetic/skills/move_base_timed.py:104-112,254-281`），
回归 fixture 位于 `aletheon-kuavo-bridge/tests/unit/test_move_base_timed.py:17-62`；Bridge 全量 pytest
为 105 passed / 6 skipped，定向 Ruff 通过。该修复在这次诊断时尚未部署，故本节的成功动作只能算
diagnostic；后续 candidate 安装态证据单独记录在 §2.9。

仿真默认 rosbag 记录使 `/root/.ros` 达到约 1.1 TB；在 operator 明确授权后先卸载
`/nodelet_rosbag`，再清理历史 bag/log/coredump，仅保留当前运行日志。清理后目录约 3.2 MB，根分区
占用从 85% 降到 21%，ROS master、MuJoCo/controller nodelets 和 Bridge 均继续运行。

### 2.2 R2 candidate 本地跨语言证据与远端复核

R2 candidate 满足 ADR 保留的 provider-attested 启动 gate：

- Rust provider 在连接阶段只读执行 Health/GetCapabilities/ListSkills，并在注册前校验 device、
  protocol digest、observation schemas、完整 descriptor schema 与非空 allowlist
  (`crates/hardware/src/grpc/provider.rs:154-265`)；
- deterministic descriptor digest 由 exact startup allowlist 计算
  (`crates/contracts/src/types/embodiment.rs:97-104`)并写入 report provenance
  (`crates/cognit/src/harness/robot/session.rs:127-128`)；
- 本地 candidate bridge 的 manifest/runtime allowlist、capability fields、Unix receive timestamp
  与 build-owned digest 已闭合；bridge pytest 90/90（6 skipped），changed-file Ruff、compileall
  以及 Rust cross-language startup gate 均通过；
- 本地跨语言日志位于 `target/r2-local-cross-language-20260806-091320/`：bridge log SHA-256
  `897dfbb8eb242b7daf97bb344896fe4a33e33b6317161b3fbd7dee31ff3785e8`，Rust provider log
  SHA-256 `da155f5056c1f298c6093cf437b4277cf0a16f12eeab621a170a4679efb1c42a`。

再次只读连接远端旧 Bridge 时，最终 candidate provider 以 typed error
`Bridge is not ready: HEALTH_STATE_UNAVAILABLE` fail closed；日志位于
`target/r2-remote-readonly-recheck-20260806-082924/provider.log`，SHA-256
`4ad5035e10d24f521671670adc22c77a718d98ef8853606a872d9dafaff5ae67`。该复核没有动作 RPC、
远端文件修改或服务重启；它证明 gate 生效，不证明远端 candidate 或 R8 已通过。

### 2.3 R3 Perception / FrameRef 本地证据

R3 按 ADR 保留的 perception/FrameRef contract 达到本地 `code_complete`：

```text
Bridge observation --Unix wall facts--> Hardware receive anchor --local MonoTime--> WorldState
        |                                                                  |
        +--typed FrameRef metadata--> bounded PerceptionStore --> Cognit Plan --> Policy
                                                    |                           |
                                                    +--> proposal/report refs --+
```

- Cognit 只依赖抽象 `RobotPerceptionPort`（`crates/cognit/src/harness/robot/mod.rs:32-40`）；
- Executive adapter 按 freshness、device、schema/source sequence、URI prefix、frame 数量和编码字节
  预算选择 refs（`crates/cognit/src/harness/robot/perception_store.rs:25-184`）；
- Hardware 保留原始 Unix captured/received time，并从 RPC receive anchor 计算 process-local MonoTime，
  不再把 epoch 毫秒直接当 monotonic time（`crates/hardware/src/grpc/convert.rs:99-166`）；
- visual payload 进入 world snapshot 前只保留 URI/digest/camera/sequence；Host 选中的 typed refs 写入
  proposal/report，原图和 base64 不进入 prompt/session/EpisodeReport
  （`crates/hardware/src/world_state.rs:146-177`、
  `crates/contracts/src/types/episode_report.rs:58-72`）。

本地验证包括 Fabric frame/SkillProposal/report、Hardware conversion、Cognit recording-policy 与
required-perception gate、Executive cache/world-state/config/session，以及 Dasein aggregator；五个相关
crate 的 all-target checks、architecture、format 和 diff checks 均通过。远端旧 Bridge 当前没有 typed
visual frame metadata，且真实 Policy 未运行，所以本节不声称 deployed/installed acceptance。

### 2.4 R4 Policy Gateway 本地协议证据

R4 按真实 Policy gateway contract 达到本地 `code_complete`：

```text
natural-language goal + fresh typed snapshots + FrameRefs + exact allowlist
                                  |
                                  v
                  real tonic Health/GetCapabilities/Propose
                                  |
                                  v
        host-bound provider/protocol + typed model/version/digest
                                  |
                                  v
       contextual validator -> semantic SkillRequest -> governed execution
```

- additive Policy wire snapshot contract 位于
  `crates/cognit/proto/aletheon/policy/gateway/v1/policy.proto:23-59`；payload 只传 typed world-state
  object，visual bytes 仍只通过 refs 表达；
- production client 只发 fresh、同 device、非空 schema/version 的 bounded snapshots，并为 timeout、空响应、
  gateway/RPC/limit/invalid response 返回 typed failure
  （`crates/cognit/src/adapters/policy/grpc_provider.rs:267-364`、`:417-475`）；
- validator 要求 outcome path 在 fresh typed snapshot 中可观察，数值 predicate 的当前 path 类型必须为数值，
  timeout/stable window 不超过 live descriptor cap，NaN/Infinity confidence 不能通过
  （`crates/cognit/src/harness/robot/proposal_validator.rs:16-235`）；
- accepted provider/protocol 来自 startup capability connection facts；response provider spoof 会失败，
  model/version/digest 来自 typed response，并随 EpisodeReport 序列化
  （`crates/contracts/src/types/episode_report.rs:58-101`）；
- `crates/aletheon/tests/robot_policy_path.rs` 的 mechanical contract 同时确认 Policy port/adapter 不包含
  embodiment execution、cancel 或 safe-stop capability。

本地证据：real in-process gRPC gateway 4/4、validator 10/10、Plan failure 5/5、Policy boundary 4/4、
WorldState freshness 4/4、Robot session 1/1、Fabric proposal/report/outcome 9/3/9，以及四个相关 crate 的
all-target、architecture、format、diff checks。该 server 是协议 fixture，不是真实 VLA 模型，也不是 installed
runtime；远端 Policy 未运行，因此不声明 direct-live 或 R8 acceptance。

### 2.5 R5 受控 replan/recovery 本地证据

R5 按有限状态恢复要求达到本地 `code_complete`：

```text
failed attempt -> typed failure + durable attempt summary
       | retry budget                    | replan budget
       v                                 v
same validated request         typed ReplanContext -> Policy
                                         |
                        validate -> authorize -> execute -> verify
                                         |
                   repeated + unchanged world -> safe stop -> failed settle
```

- retry/replan 独立计量，只有实际进入 retry execution / Policy replan 时才消费 budget
  （`crates/cognit/src/harness/robot/mod.rs:681-830`）；
- replan wire 仅携带 typed failure、attempt digest、world schema/version/sequence 和剩余 budget，仍经过
  proposal validator 与 authorization（`crates/cognit/proto/aletheon/policy/gateway/v1/policy.proto:23-62`、
  `crates/cognit/src/harness/robot/state.rs:98-123`）；
- provider disconnect、unsafe/unknown evidence、budget exhaustion 和 cancellation 都进入 safe-stop + failed
  settlement；safe-stop/settlement failure 追加到 ordered failure history，不覆盖 primary cause
  （`crates/cognit/src/harness/robot/mod.rs` 中的 `route_replan_or_stop`、`safe_stop_and_close` 与 `step`）；
- Robot session 从 durable sink 重建 ordered attempt list；pre-cancelled turn 也返回 typed failed EpisodeReport，
  不丢弃恢复证据（`crates/cognit/src/harness/robot/session.rs` 中的 `build_report` 与 `run_turn`）。

本地证据：R5 recovery 5/5、Policy gRPC 5/5、state machine 9/9、Executive robot-session E2E
3/3、Fabric failure/report 1/3；Fabric/Cognit/Executive all-target、architecture、format、diff checks
均通过。没有真实 Policy/Bridge/ROS/MuJoCo 或 installed runtime 运行，因此本节不构成 R8 acceptance。

### 2.6 R6 实机安全 capability 本地证据

R6 的本地关闭范围对应 ADR 保留的 safety capability 与验收条件：

```text
provider-owned environment + safety manifest
                   |
          Aletheon bootstrap gate
                   |
  pinned HIL/real serial + manifest + limits + evidence
                   |
     request-bound high-risk approval -> Kernel permit

Aletheon heartbeat --control session--> Bridge local monotonic watchdog
                                              |
                         lease/owner/deadline loss -> local safe stop
physical/local estop -----------------------------> direct stop callback
```

- simulation/HIL/real 与 safety facts 来自 provider handshake，不由 operator config 升级；real 缺任一
  emergency/joint/velocity/torque-or-current/ownership/safe-stop/independent-hard-stop capability 即 bootstrap
  fail closed（`crates/contracts/src/types/embodiment.rs:17-167`、
  `crates/hardware/src/grpc/provider.rs:504-568`）；
- HIL/real profile 必须 pin device serial、canonical safety manifest digest 与 driver limits digest；real
  还要求未过期 evidence 和非 loopback TLS endpoint。live snapshot 在 provider 注册前逐项比较
  （`crates/aletheon/src/config/robot.rs:438-632`、
  `crates/aletheon/src/wiring/daemon/bootstrap/embodiment.rs:85-237`）；
- high-risk skill 只接受与 principal/device/skill/完整参数 digest/expiry 绑定的 operator receipt，默认 adapter
  拒绝；approval schema 不含 VLA confidence（`crates/hardware/src/approval.rs:10-85`）；
- Bridge 用本地 monotonic clock 管理独占 owner、heartbeat、operation deadline 与 lease；daemon/heartbeat
  消失不等待 Agent/LLM。驱动 movement handler 对 compiled/deployment/reviewed 三层最小限制执行 reject，
  不 clamp（bridge `watchdog.py:44-190`、`move_base_timed.py:18-112`）；
- local emergency-stop latch 直接调用 device callback，不经过 gRPC、Policy 或 LLM（bridge
  `safety.py:78-107`、`aletheon-kuavo-bridge/tests/fault_injection/test_safety.py:129-140`）。

本地证据：Bridge pytest 101 passed/6 skipped；Fabric embodiment 7/7；Hardware 72 passed/3 ignored；
Kernel 72/72；Executive profile 11/11、approval/bounds 4/4 与 pin/receipt tests；Cognit validator
11/11 + integration 2/2；local fake-Bridge Rust startup diagnostic 1/1。proto 两份逐字节相同，SHA-256
为 `77e44869ba6a6b3345753f699f9acd403a6c8f4de3383dcd5827b0ba7673e679`。这些是 local/code-complete
证据；没有物理独立硬停、HIL/real device 或 installed runtime 证据，不得描述为 real-ready。

### 2.7 R7 Episode artifact/report 本地证据

R7 对应 artifact manifest、外部引用、attempt/sequence、settlement、crash recovery、immutable
promotion 与 retention tombstone：

```text
ordered attempts + external evidence refs
                  |
                  v
        immutable settled receipt --digest--> audit / promotion gate
                  |
       append-only retention tombstone
                  |
                  v
     read projection: evidence retention expired
```

- `EpisodeReport` 保存 model/bridge/scene/descriptor provenance、每次 attempt 的 operation ID 与
  before/after/verified sequence 以及 verifier 实际读取的 predicate paths；`EpisodeSettlement` 是 report 与 `TurnStop` 的同一权威，
  `SettledEpisodeReport` 以 SHA-256 绑定不可变 receipt
  （`crates/contracts/src/types/episode_report.rs` 中的 `EpisodeSettlement`、`AttemptRecord`、`EpisodeReport` 与
  `SettledEpisodeReport`）；
- frame 转换为完整 manifest；旧 provider 只有 URI 时显式标记 metadata incomplete，不伪造
  digest/size/producer/time range，并拒绝大小写变体的 inline/data URI
  （`crates/contracts/src/types/episode_report.rs:87-325`）；
- session 从 SQLite 重建全部重试 evidence，然后按 persist→audit→promotion 消费同一不可变
  receipt；失败 episode 保留 evidence 但不进入成功经验
  （`crates/cognit/src/harness/robot/session.rs` 中的 `build_report` 与 `run_turn`、
  `crates/mnemosyne/src/episode_promotion.rs:24-50`）；
- fresh SQLite schema 仅保存有界的 expected/result/verification JSON 和 sequence，大 artifact 只保存
  manifest/reference；report 与 tombstone 表有 immutable trigger
  （`crates/adapters/sqlite/src/migrations/001_episodes.sql:7-86`）；
- retention 先持久化 tombstone 再删除 bytes，重启清理“已提交 tombstone/尚未删文件”的 crash
  窗口，且拒绝同 digest 复活；若进程在 local artifact expiry 与 episode projection 写入之间退出，
  下一次带 ArtifactStore 的权威读取按 digest 补写 tombstone。report 只以读时 projection 显示 evidence 已过期，不修改 receipt
  （`crates/adapters/sqlite/src/artifact.rs:273-430`、
  `crates/adapters/sqlite/src/episode.rs:267-428`）。

本地验证：Fabric EpisodeReport 9/9；Executive verifier 6/6、SQLite sink 8/8、artifact retention
1/1、promotion 3/3、audit 5/5、Robot session E2E 3/3；Cognit robot
harness/perception/replan/proposal/Policy targets 27/27。Fabric/Cognit/Executive all-target checks、
architecture（22 migrations / 62 acceptance IDs / 1079 Fabric public types）、fmt 和 diff check 通过。这些不是
installed/runtime acceptance。

R8 的 safe-stop 证据不再由 failed/cancelled settlement 推断。状态机在 safe-stop executor
返回后记录 `SafeStopReceipt` 的 attempt、typed trigger 与 succeeded/failed outcome；report
验证 outcome 与 `SafeStopFailure` 历史一致，audit 只消费该 typed fact
（`crates/contracts/src/types/episode_report.rs` 中的 `SafeStopReceipt` 与 `EpisodeReport::validate`、
`crates/cognit/src/harness/robot/mod.rs:493-511`、
`crates/cognit/src/harness/robot/audit_chain.rs:77-103`）。Fabric contract、Cognit recovery 和
Executive cancellation session 已覆盖成功/失败 safe-stop receipt；这仍不是远端真实 safe-stop 证据。

### 2.8 R8 安装态证据检查器（只读）

完成 operator 授权的 candidate 部署、正向/负向真实任务以及 daemon 重启后，可用统一入口对
`/usr/bin/aletheon` 保存的原始 JSON 输出和重启后 SQLite 做只读交叉验证：

```bash
bash scripts/aletheon.sh acceptance robot-r8 \
  --report /path/to/official-client-episode-report.json \
  --database /path/to/robot-episodes.db \
  --expected-device kuavo-mujoco-01 \
  --expected-settlement completed \
  --minimum-stable-window-ms 3000 \
  --expected-scene kuavo-mujoco/default-v40 \
  --expected-bridge-digest "$EXPECTED_BRIDGE_DIGEST" \
  --expected-skill-descriptor-digest "$EXPECTED_SKILL_DIGEST" \
  --expected-policy-provider "$EXPECTED_POLICY_PROVIDER" \
  --expected-policy-model "$EXPECTED_POLICY_MODEL" \
  --expected-policy-version "$EXPECTED_POLICY_VERSION" \
  --expected-policy-protocol "$EXPECTED_POLICY_PROTOCOL" \
  --expected-policy-digest "$EXPECTED_POLICY_DIGEST" \
  --output /path/to/r8-positive-receipt.json

# 负向任务额外要求真实 safe-stop terminal receipt：
bash scripts/aletheon.sh acceptance robot-r8 \
  --report /path/to/official-client-negative-report.json \
  --database /path/to/robot-episodes.db \
  --expected-device kuavo-mujoco-01 \
  --expected-settlement failed \
  --minimum-stable-window-ms 3000 \
  --require-safe-stop \
  --expected-safe-stop-outcome succeeded
```

检查器要求每次 attempt 都有 operation ID、predicate、至少 3 秒 stable window、实际 observed
paths、before/after/verified sequence 和 terminal verification；它以 `mode=ro`/`query_only`
打开数据库，逐项比较 immutable receipt、report digest、attempt 顺序、episode state，并执行
`PRAGMA integrity_check`（`scripts/libexec/aletheon/robot_r8_evidence.py:76-297`）。fixture tests
覆盖 bounded receipt、completed、safe-stop negative、弱 attempt evidence 和 durable digest drift 5/5
（`scripts/tests/test_robot_r8_evidence.py:138-237`）。

直接运行 Python 检查器只会输出 `REPORT_EVIDENCE_PASS`，不会启动、停止、重启或配置
Aletheon/Policy/Bridge/ROS/MuJoCo，也不检查 daemon executable digest 和 systemd restart counter。
公共入口 `aletheon.sh acceptance robot-r8` 会先执行 system-installed provenance、稳定性、Memory
Agent 和 official-socket gate，再运行只读 report/SQLite 检查器
（`scripts/lib/aletheon/acceptance.sh:10-16`、`scripts/lib/aletheon/runtime_gate.sh:46-193`）。这些
本地 fixture 只能证明检查器行为；任何新的安装态验收仍必须同时提供实际正向/负向 Robot task evidence。

### 2.9 2026-08-07 R8/X13 安装态 MuJoCo 验收

PR #182 合入前，installed release、`/usr/bin/aletheon`、machine core、user daemon 与 Memory Agent
的 SHA-256 均为 `038986c29aaf37680d0a8ce0702235bbf50c13b4df8aac9dd73be80730007b1a`；systemd restart
counter 稳定，official user socket 的真实 LLM 请求通过。该 digest 是当次验收 provenance，不表示
后续版本应继续使用同一二进制。

真实链路如下：

```text
/usr/bin/aletheon -> official user socket -> installed daemon
  -> RobotCognitiveSession -> aletheon-vla-lejurobot/deepseek-v4-flash
  -> Kernel/Hardware -> Bridge 77e44869... -> ROS -> kuavo-mujoco/biped-s53
  -> stable verifier -> immutable EpisodeReport / SQLite receipt
```

- Policy protocol `1.0`，Bridge protocol digest 为
  `77e44869ba6a6b3345753f699f9acd403a6c8f4de3383dcd5827b0ba7673e679`；
- 三次连续真实 TUI stance run 都执行唯一 operation-bound `kuavo.stance {}`，对所有 base-twist
  path 连续稳定验证 3000 ms，并 settle `completed`；
- 受控正向 `kuavo.move_base_timed` 产生 ground-truth displacement、完成验证、回到零速并 settle
  `completed`；
- unsafe/unavailable 负向请求由 Policy 返回 typed `SafetyFallback`，Host 记录
  `proposal_rejected`，执行 attempt 数为 0，terminal `safe_stop` 成功，settlement 为 `failed`；
- `scripts/libexec/aletheon/robot_r8_evidence.py` 逐项核对 request/attempt/operation、sequence、stable
  window、provenance、artifact 和 safe-stop 与 SQLite；正向和负向 receipt 均为
  `REPORT_EVIDENCE_PASS`，daemon restart 后再次读取仍一致。

当次汇总和 immutable reports 位于
`target/r8-x13-installed-20260806-174928/x13-current-sha-20260807-003254/`。该目录是本地保留的
验收 artifact，不是运行时 authority。R8/X13 的 accepted 结论仅覆盖 MuJoCo；物理 HIL/real 仍受
ADR 的独立安全 gate 约束。

## 3. 测试 artifact 目录与清理

- 单测/集成测试使用临时目录（tempfile），测试结束删除。
- live bridge diagnostic 的 rosbag/frame/log 落到 operator config 指定的 artifact root，只保留 digest/引用，不内联进 session/event/数据库。
- 保留策略目标是成功 evidence 30 天、失败 artifact 90 天并保留 tombstone；R7 已实现可重试的删除/tombstone/report projection 生命周期，但未验证 robot artifact 的定时 job、生效配置或 30/90 天安装态执行，因此不得把该期限描述为当前已生效策略。

## 4. 本轮"已完成"能力 → 证据锚点（必须能追到类型/测试）

| 能力 | 锚点 |
|---|---|
| Embodiment domain contract | `fabric::types::embodiment`、`expected_outcome`、`outcome_verification`、`episode_report` |
| gRPC bridge 协议/provider | `crates/hardware/proto/aletheon/embodiment/gateway/v1/gateway.proto`、`hardware::grpc::provider` |
| Kernel admission/authority | `executive::application::embodiment_authority` |
| Robot state machine + validator | `cognit::harness::robot`、`proposal_validator` |
| Production bootstrap | `executive::host::daemon::bootstrap::robot` |
| Policy gRPC client | `cognit::adapters::policy::grpc_provider` |
| World state + verifier | `executive::application::world_state`、`deterministic_outcome_verifier` |
| Durable attempts/report | `sqlite_episode_sink`、`RobotCognitiveSession` |
| Immutable episode audit/retention | `SettledEpisodeReport`、`robot_audit`、`ArtifactStore::expire_retention`、`load_report_projection` |
| Spine progress | `embodiment_progress` + bootstrap binding |
| Mnemosyne promotion | `robot_episode_promotion`（`SettledEpisodeReport::can_promote()` integrity gate） |
| Real bridge execute E2E | `executive/tests/robot_bridge_execute_e2e.rs`（`#[ignore]`；历史 diagnostic 报告通过，非本轮 acceptance） |

## 5. 尚未验收的独立范围

- 物理实机/HIL：独立 R6 gate，必须在真实设备上证明 pinned serial/manifest/limits、local
  watchdog/ownership、independent hard stop 与 emergency-stop；MuJoCo R8/X13 不能替代。
- robot artifact 的 30/90 天定时 retention job 与实际部署策略尚未完成安装态时间跨度验证。
