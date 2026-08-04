# Robot 运行时与仿真测试基线（R0）

> 依据：`docs/plans/robot-vla-production-closure-plan.md` Phase R0
> 基线：`origin/dev@c080b08bf3170dd8a09acdb738a255133abbf11b`（2026-08-05）
> 本轮约束：**暂不要求物理实机验证**；历史仿真运行记录只能作为背景，不作为本次 acceptance evidence。
> R0 状态：`in_progress`；bridge commit/version、proto digest、scene version 尚未采集。

## 1. 外部依赖（本仓库不复制源码）

公开仓库只保存版本化事实和占位符，不保存公司内网用户名、IP、绝对路径、隧道命令或凭据。实际访问参数由受控 operator config 提供。

| 能力 | 位置/入口 | 当前版本事实 |
|---|---|---|
| Kuavo bridge | `<robot-host>/<bridge-workspace>`；loopback gRPC endpoint 由部署配置解析 | repository URL/commit：`NOT_RECORDED`；补齐前 R0 不可 accepted |
| Bridge protocol | `crates/hardware/proto/aletheon/embodiment/gateway/v1/gateway.proto` 与外部生成物 | proto digest：`NOT_RECORDED` |
| ROS runtime | `<robot-host>/<ros-workspace>` | distro/package lock：`NOT_RECORDED`；历史环境为 Ubuntu 20.04/Python 3.8，仅作背景 |
| MuJoCo scene | `<robot-host>/<sim-workspace>` | scene ID/version/digest：`NOT_RECORDED` |
| 设备标识 | 部署配置中的 `device_id` | 历史 diagnostic 使用 `kuavo-mujoco-01`；本轮须从 resolved config 重新记录 |
| 连接方式 | operator config 管理的受控 tunnel/endpoint | 命令和地址不入库；evidence 只记录 endpoint identity digest |

R0 转为 `accepted` 前，必须把所有 `NOT_RECORDED` 替换为可验证版本/digest；不得把“已部署”当作版本证据。

## 2. 测试分层（禁止把 `#[ignore]` live test 的存在当成当次已通过）

| 层 | 入口 | 是否本轮验收 | 说明 |
|---|---|---|---|
| deterministic unit/contract | fabric/cognit/executive 定向测试 | 是 | 状态机、validator、verifier、episode sink、config |
| in-process robot session | `robot_session_e2e`、`robot_harness_composition` | 是 | Turn→EpisodeReport 链路，无外部依赖 |
| direct live bridge diagnostic | `robot_bridge_execute_e2e --ignored` | 仅 diagnostic | 需远程 bridge+MuJoCo；**不替代 R8 安装态验收** |
| installed full runtime | `/usr/bin/aletheon` official socket + 真实 Policy/bridge/MuJoCo | R8 | `not_started`；preflight 证明 Policy/Bridge/MuJoCo 任一缺失后才可 `externally_blocked`；物理实机不是前置 |

## 3. 测试 artifact 目录与清理

- 单测/集成测试使用临时目录（tempfile），测试结束删除。
- live bridge diagnostic 的 rosbag/frame/log 落到 operator config 指定的 artifact root，只保留 digest/引用，不内联进 session/event/数据库。
- 计划目标是成功 evidence 30 天、失败 artifact 90 天并保留 tombstone；R7 在实现 retention job、配置和删除证据前，不得把该期限描述为当前已执行策略。

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
| Spine progress | `embodiment_progress` + bootstrap binding |
| Mnemosyne promotion | `robot_episode_promotion`（`EpisodeReport::can_promote()` gate） |
| Real bridge execute E2E | `executive/tests/robot_bridge_execute_e2e.rs`（`#[ignore]`；历史 diagnostic 报告通过，非本轮 acceptance） |

## 5. 明确不在本轮验收

- 真实 Policy/VLA gateway（R4 无真实 gateway → `code_complete`）。
- 物理实机（独立 R6/HIL gate，不是 R8 MuJoCo 关闭条件）。
- 官方安装态全链路（R8：release/`/usr/bin/aletheon` 与所有运行中 Aletheon daemon executable SHA 一致 + official socket 真实 Policy/Bridge/MuJoCo）。
