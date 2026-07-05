# Aletheon 生产化路线图

**Date**: 2026-07-05
**Status**: Draft
**输入**: 外部 review (`tmp.md`, 12 issues) + TUI 重构 spec

## 总览

```
Phase 1: 安全边界（P0，阻塞生产部署）
  ├── 1.1 Socket 权限 → SO_PEERCRED
  ├── 1.2 非 root 服务用户
  ├── 1.3 --sandbox 参数修复
  └── 1.4 自演化 PermissionManager 闸门

Phase 2: 发布安装（P1，阻塞 CI/部署）
  ├── 2.1 Release CI 修复
  ├── 2.2 统一 service 定义（删除旧模板）
  └── 2.3 setup.sh 精准进程管理

Phase 3: 最小产品范围（含 TUI 重构）
  ├── 3.1 TUI 重构（四层架构）
  ├── 3.2 README 产品定位修正
  └── 3.3 内部迁移收尾（EventBus → CommunicationBus）

Phase 4: 故障测试
  ├── 4.1 受限环境测试跳过机制
  └── 4.2 生产验收测试矩阵

Phase 5: 高级能力（审批闸门完成后才开放）
  ├── 5.1 自演化完整自动化
  ├── 5.2 内核工具 / 跨平台
  └── 5.3 离线本地模型
```

---

## Phase 1: 安全边界

### 1.1 Socket 权限 + 客户端身份校验

**现状** (`server.rs:34-39`):
```rust
let listener = UnixListener::bind(socket_path)?;
// Allow all users to connect
std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o666))?;
```

任何本机用户都可以连接 daemon 并执行任意 shell 命令。这是最大风险。

**方案**:

1. Socket 权限改为 `0660`，创建专用 `aletheon` 组
2. Accept 时校验 `SO_PEERCRED`，只允许 socket 文件 owner 和同组成员连接
3. 按 RPC method 分级权限（将来可扩展为细粒度 ACL）：

| RPC Method | 最低权限 |
|-----------|---------|
| `chat`, `session.ask` | 同组成员（普通交互） |
| `session.reflect`, `session.snapshot` | 同组成员 |
| `session.evolution.trigger` | socket owner only（自演化通道） |
| `debug.*` | socket owner only |

```rust
// server.rs — new accept path
fn check_peer_cred(stream: &UnixStream, owner_uid: u32, group_gid: u32) -> Result<()> {
    let cred = stream.peer_cred()?;
    if cred.uid() == 0 || cred.uid() == owner_uid { return Ok(()); } // root or owner
    let groups = getgrouplist_for_user(cred.uid(), cred.gid())?;
    if groups.contains(&group_gid) { return Ok(()); }
    Err(anyhow!("Access denied: uid {} not authorized", cred.uid()))
}
```

**setup.sh 配套变更**: 创建 `aletheon` 系统用户和组，把实际用户加入该组。

**文件**:
| 文件 | 动作 |
|------|------|
| `crates/runtime/src/impl/daemon/server.rs` | 修改 — 0666→0660，accept 时校验 SO_PEERCRED |
| `config/aletheon.service` | 修改 — 添加 `Group=aletheon` |
| `setup.sh` | 修改 — 创建 `aletheon` 用户/组 |

### 1.2 非 root 服务用户

**现状**: `config/aletheon.service` 没有 `User=`，systemd 默认以 root 运行。虽然 `NoNewPrivileges=yes` 已设置，但 root daemon + shell 执行权限的组合仍然风险极高。

**方案**: 创建专用 `aletheon` 系统用户，service 以 `User=aletheon` 运行。daemon 需要读写：
- `/run/aletheon/` — `RuntimeDirectory=aletheon`（systemd 自动管理）
- `/var/lib/aletheon/` — `StateDirectory=aletheon`（systemd 自动管理）
- `~/.aletheon/.env` — 不再需要（daemon 用自己的配置目录）

HOME 不再指向 SUDO_USER。daemon 的 `~/.aletheon/` 使用 `/var/lib/aletheon/`。

用户级别的 `.env`（API key）通过 `/etc/aletheon/.env` 注入（`EnvironmentFile` 已存在，只需把 key 放那里）。

**setup.sh 变更**: 移除 HOME 模板变量，统一使用 `/etc/aletheon/.env` 作为配置来源。

**文件**:
| 文件 | 动作 |
|------|------|
| `config/aletheon.service` | 修改 — 添加 `User=aletheon`，移除 `HOME=__HOME__` |
| `setup.sh` | 修改 — 去掉 HOME 模板生成，确保 `/etc/aletheon/.env` 包含 API key |

### 1.3 --sandbox 参数真正生效

**现状** (`main.rs:65-67` + `main.rs:251-255`):
```rust
// CLI 定义
#[arg(long, default_value = "auto")]
sandbox: String,
// exec 模式中：参数被命名为 _sandbox，未被使用
// 始终调用 ToolRunnerWithGuard::with_default_sandbox()
```

用户传入 `--sandbox require` 时仍可能降级执行，安全契约不一致。

**方案**: 将 `--sandbox` 参数传递给 `ToolRunnerWithGuard`：

```rust
// SandboxPolicy 枚举
pub enum SandboxPolicy {
    Auto,     // 有 bwrap 则用，否则降级
    Require,  // 必须有 bwrap，否则报错退出
    Forbid,   // 不用 sandbox
}

// 传入 runner
let policy = SandboxPolicy::from_str(&cli_sandbox)?;
let mut runner = ToolRunnerWithGuard::with_sandbox_policy(
    AuditLogger::new(audit_path)?,
    policy,
);
```

**文件**:
| 文件 | 动作 |
|------|------|
| `crates/aletheon/src/main.rs` | 修改 — `--sandbox` 传入 runner |
| `crates/corpus/src/security/sandbox/mod.rs` | 修改 — 添加 `SandboxPolicy` 枚举 |

### 1.4 自演化闸门

**现状** (`evolution_coordinator.rs:161`):
```rust
// HIGH-risk autonomy gate. TODO(Tier 2a): also require PermissionManager approval.
if !self.config.enabled { ... }
```

目前只有 `config.enabled` 这一个开关，缺独立的 `PermissionManager` 审批。

**方案**: 本阶段先加 CLI flag 闸门（最小改动），后续 Phase 5 再实现完整 PermissionManager：

```rust
// 新增：自演化需要 --enable-evolution CLI flag 且 config 也启用
if !self.config.enabled || !ctx.evolution_permitted {
    return Ok(EvolutionSummary { evolution_triggered: false, ... });
}
```

daemon 启动时 `evolution_permitted` 默认 `false`，仅当显式传 `--enable-evolution` 才启用。

**文件**:
| 文件 | 动作 |
|------|------|
| `crates/aletheon/src/main.rs` | 修改 — daemon 子命令添加 `--enable-evolution` flag |
| `crates/runtime/src/core/evolution_coordinator.rs` | 修改 — 加 `evolution_permitted` 检查 |
| `crates/runtime/src/impl/daemon/handler/init.rs` | 修改 — 传入 flag 值 |

---

## Phase 2: 发布安装

### 2.1 Release CI 修复

**现状** (`.github/workflows/release.yml`):
要求 `-p aletheond -p aletheon-exec -p aletheon-cli`，但这些 package 已不存在。当前唯一的二进制 package 是 `crates/aletheon/Cargo.toml` (`name = "aletheon"`)。

**方案**: 改为 `cargo build -p aletheon --release`，artifact 名改为 `aletheon`。

**文件**:
| 文件 | 动作 |
|------|------|
| `.github/workflows/release.yml` | 修改 — package 名从三个旧名改为 `aletheon` |
| `.github/workflows/ci.yml` | 检查 — 确认 ci 已使用正确 package 名 |

### 2.2 统一 Service 定义

**现状**: 三套互相矛盾的 service 文件：

| 文件 | 程序名 | Socket | User |
|------|--------|--------|------|
| `config/aletheon.service` | `aletheon daemon` | `/run/aletheon/aletheon.sock` | root（模板化） |
| `config/aletheon.user.service` | `aletheon daemon` | — | 当前用户 |
| `systemd/aletheond.service` | `aletheond` | `/run/aletheond/aletheond.sock` | root |

**方案**: 删除旧文件，只保留两套（对应两个安装模式）：

- `config/aletheon.service` — system 模式，`User=aletheon`
- `config/aletheon.user.service` — user 模式，`systemctl --user`

删除 `systemd/aletheond.service`（旧二进制名），删除 `systemd/` 目录。

**文件**:
| 文件 | 动作 |
|------|------|
| `config/aletheon.service` | 修改 — 去掉 `__HOME__`/`__PROJECT_DIR__`，加 `User=aletheon`/`Group=aletheon` |
| `config/aletheon.user.service` | 保留，可能微调 |
| `systemd/aletheond.service` | **删除** |
| `setup.sh` | 修改 — 适配新 service 内容 |

### 2.3 setup.sh 精准进程管理

**现状** (`setup.sh:124-163`):
```bash
for proc in aletheond aletheon-exec aletheon-systemd aletheon-container aletheon; do
    pids=$(pgrep -f "$proc" 2>/dev/null || true)
    if [[ -n "$pids" ]]; then
        echo "$pids" | while read pid; do
            kill "$pid" 2>/dev/null || true
        done
    fi
done
```

`pgrep -f aletheon` 匹配任何包含 "aletheon" 字符串的进程。

**方案**: 只管理已知的 daemon 进程：

```bash
# 只 stop systemd service，不扫进程
if systemctl is-active --quiet aletheon.service 2>/dev/null; then
    systemctl stop aletheon.service
fi
# 如果 systemd 不可用（容器/Docker），用 pidfile
if [[ -f /run/aletheon/aletheon.pid ]]; then
    kill "$(cat /run/aletheon/aletheon.pid)" 2>/dev/null || true
fi
# 只清理已知 socket，不杀进程
rm -f /run/aletheon/aletheon.sock
```

**文件**:
| 文件 | 动作 |
|------|------|
| `setup.sh` | 修改 — 用 systemctl stop + pidfile 替代 pgrep 扫进程 |

---

## Phase 3: 最小产品范围

### 3.1 TUI 重构（四层架构）

这是之前 spec 中已详细描述的部分，在此整合为子阶段。

**动机**: 
- TUI 直接输出 JSON-RPC 裸协议到终端
- 工具调用输出污染聊天区
- daemon ↔ client 协议是隐式约定（靠字符串匹配同步）
- `base::UiEvent` 是死代码

**架构**:

```
Layer 0: 共享协议 (base::ClientEvent，替代死代码 UiEvent)
  daemon: Event → ClientEvent → serde_json → socket
  client: socket → serde_json → ClientEvent → ChatWidget typed handlers

Layer 1: 显示模型 (HistoryCell trait)
  UserMessageCell | AgentMessageCell | ExecCell | PlanCell | ApprovalCell | StatusEventCell

Layer 2: 布局系统 (Renderable trait + FlexRenderable)
  draw.rs 从手动拼装改为 renderable 树组合

Layer 3: 流式渲染 (StreamCore)
  stable/tail 双区 + 表格 holdback + commit 动画队列
```

**受影响文件**: 27 个（15 新建，5 修改，5 删除，2 替换）

详细设计见 `docs/plans/2026-07-05-tui-redesign.md`。

**实现阶段**: 7 步
1. Phase 0: `ClientEvent` 在 base，`From<Event>` 在 format.rs（wire format 不变）
2. Phase 1: `ChatWidget::handle_event(match ClientEvent)` 替代字符串 dispatch
3. Phase 2: `Renderable` trait + composites，重构 `draw.rs`
4. Phase 3: `HistoryCell` + `ChatWidget` 替代 `ChatMessage`
5. Phase 4: `StreamCore` + commit animation + table holdback
6. Phase 5: Markdown 表格渲染、语法高亮、fence unwrap
7. Phase 6: 删除 `response.rs`、`toolcard.rs`、`approval_dialog.rs`；dedup `cli.rs`

### 3.2 README 产品定位修正

**现状** (review #8): README 声称的 "跨平台、离线、内核感知" 多数仍是设计/实验阶段。

**方案**: 在 capabilities 部分添加状态标签：

| 能力 | 状态 |
|------|------|
| Linux 本地 Agent Runtime | ✅ 可用 |
| TUI + CLI 交互 | ✅ 可用 |
| 多模型支持 (Anthropic / OpenAI 兼容) | ✅ 可用 |
| Bubblewrap 沙箱工具执行 | ✅ 可用 |
| 多 Agent 协作 | ✅ 可用 |
| 本地离线模型 | 🔧 实验阶段 |
| 自演化 / 自动部署 | 🔧 需显式启用 |
| Android / 嵌入式 | 📋 设计阶段 |
| eBPF 内核感知 | 📋 设计阶段 |
| 跨平台 (macOS / Windows) | 📋 设计阶段 |

**文件**:
| 文件 | 动作 |
|------|------|
| `README.md` | 修改 — 添加状态标签 |

### 3.3 内部迁移收尾

**现状** (review #10): EventBus → CommunicationBus 迁移有 4 个 TODO (P1-A) 标在 `dasein/event_bridge.rs`、`handler/mod.rs`、`handler/chat.rs`、`agent/fork.rs`。

**方案**: 本阶段只完成 P1-A 迁移（去掉 `#[allow(deprecated)]`，统一用 CommunicationBus）。其他 feature stub（io_uring、LanceDB、bottleneck_detector）保留不动。

**文件**:
| 文件 | 动作 |
|------|------|
| `crates/dasein/src/dasein/event_bridge.rs` | 修改 — EventBus → CommunicationBus |
| `crates/runtime/src/impl/daemon/handler/mod.rs` | 修改 — 同上 |
| `crates/runtime/src/impl/daemon/handler/chat.rs` | 修改 — 同上 |
| `crates/runtime/src/impl/agent/fork.rs` | 修改 — 同上 |

---

## Phase 4: 故障测试

### 4.1 受限环境测试跳过

**现状** (review #11): 4 个 Unix socket bind 测试在沙箱/容器中失败（`Operation not permitted`）。

**方案**:
1. 添加测试 feature gate `#[cfg_attr(not(feature = "network-tests"), ignore)]`
2. CI 中 `cargo test --workspace` 默认跳过需要网络/socket 权限的测试
3. 提供 `cargo test --workspace --features network-tests` 用于完整环境

**文件**:
| 文件 | 动作 |
|------|------|
| `crates/base/Cargo.toml` | 修改 — 添加 `network-tests` feature |
| `crates/base/src/ipc/backends/unix_socket.rs` | 修改 — 测试加 feature gate |
| `crates/base/src/ipc/transport/unix_socket_transport.rs` | 修改 — 测试加 feature gate |
| `.github/workflows/ci.yml` | 检查 — 确认测试命令适配 |

### 4.2 生产验收测试矩阵

**现状** (review #12): 缺少真实生产环境的验证测试。

**方案**: 在 `tests/` 目录添加集成测试，CI 中按 feature gate 分类运行：

| 测试类别 | Feature gate | 环境要求 |
|---------|-------------|---------|
| 单元测试 | 默认 | 无 |
| Socket 权限测试 | `network-tests` | 无沙箱 |
| API 端到端测试 | `live-api` | 有效的 API key |
| Daemon 重启恢复 | `daemon-tests` | 本地 daemon |
| 长时间运行 | `stress-tests` | 手动触发 |

测试 checklist：
- [ ] API 超时、限流、断流和重试
- [ ] Daemon 重启后会话恢复
- [ ] Socket 越权访问被拒绝
- [ ] root/普通用户安装测试
- [ ] 磁盘满时审计写入不崩溃
- [ ] 数据库损坏时优雅降级
- [ ] 工具逃逸与提示注入安全测试
- [ ] 长时间运行内存增长监测（>1h）

---

## Phase 5: 高级能力

Phase 5 只在 Phase 1 安全边界完全封闭 + Phase 4 故障测试通过后开放。

### 5.1 自演化完整自动化

在 Phase 1.4 的 CLI flag 闸门基础上：
- 实现 PermissionManager 审批管线（替代当前 flag-only 方案）
- 自演化操作的 audit log 完整链路
- 回滚机制（演化失败自动恢复上一版本）

### 5.2 内核工具 / 跨平台

- eBPF kernel introspection driver
- FUSE 文件系统工具
- Android / Embedded target support

### 5.3 离线本地模型

- Ollama 本地模型作为一等 provider
- 自动模型下载 + 量化选项
- 纯离线模式 `--offline` flag

---

## 实现顺序

```
Phase 1 ──► Phase 2 ──► Phase 3 ──► Phase 4 ──► Phase 5
 (P0)       (P1)       (P1/P2)     (P2)        (未来)
```

**Phase 1 必须先做**：socket 权限和 sandbox 参数是安全契约，不解决就不能声称可以在多用户环境运行。

**Phase 2 紧随其后**：Release CI + 统一 service 是安装链路的基础。setup.sh 的进程管理修复保证安装安全。

**Phase 3 在安全边界就绪后推进**：TUI 重构可以开始 Phase 0（ClientEvent 共享协议），因为 wire format 不变，不影响正在运行的系统。其余阶段逐步推进。

**Phase 4 与 Phase 3 并行**：测试基础设施改进和 TUI 重构可以同时进行（不同文件，无冲突）。

**Phase 5 是长期目标**：需要 Phase 1 + 4 完成（安全 + 测试覆盖）才能安全开放。

## 与之前 TUI Spec 的关系

之前的 `2026-07-05-tui-redesign.md` 现在作为 Phase 3.1 的详细实现规范引用。本文件是上层路线图。

Phase 0（ClientEvent 共享协议）从 TUI spec 上升为 Phase 1 的子任务 — 安全边界需要协议类型来确保 daemon ↔ client 通信的一致性。
