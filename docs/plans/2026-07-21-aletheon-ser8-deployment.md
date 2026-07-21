# Aletheon 实机部署与 Pi 闭环点火（SER8）

> 日期：2026-07-21
> 主机：`aurobear-SER8`，Ubuntu 24.04.4 LTS，x86_64，16 core / 27G RAM / 901G free
> 源码：`/home/aurobear/Workspace/aletheon` @ `666fcd6`（与 `origin/dev` 一致）
> 目标：把 aletheon 作为常驻服务真实跑起来（core + 用户 daemon），用 Leju deepseek 干活，
> 并点火 pi 集成，最终实现"定时 pi 任务 → 总结记忆 → gbrain"闭环。

---

## 0. 背景与拓扑（两个项目、一个共享记忆）

- **aletheon**：Rust 常驻 Agent 运行时（本仓库）。16 crates，systemd 原生部署。
- **aurb**（`/home/aurobear/Workspace/work/aurb`）：provider-neutral 资产层（skills/agents/hooks/MCP），
  被 Claude Code / Codex 消费。**非运行时**。
- **gbrain**：共享记忆服务，native `bun` 进程监听 `127.0.0.1:3131`（aurb 与 aletheon 共用）。
- 说明：另一台机器（Codex，`/workspace/scratch/.../repos/aletheon`）上有一批**未提交**改动
  （重写 setup.sh、configure-pi.sh、smoke-pi-service.sh、git 工具注册、内置 profile 等）。
  **本次全部从已提交 `666fcd6` 出发**，本机这份为规范来源；需要的修复在本机实现并编译验证。

部署模型（`docs/deployment/systemd.md`）：

```
root  aletheon-core.service   → /run/aletheon/core.sock (0660 aletheon 组)  [机器级推理核心]
              ▲
user  aletheon.socket → aletheon.service (aletheon daemon)  [Pi/Goal/memory worker 都在此]
              ▲
        TUI / 定时任务客户端
```

---

## 1. 环境（本机真实安装）✅

```bash
sudo apt-get install -y pkg-config libssl-dev sqlite3 build-essential   # bwrap/gcc/git/rg/jq/node 已在
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y \
  --default-toolchain 1.88.0 --profile minimal -c clippy -c rustfmt
npm i -g @earendil-works/pi-coding-agent      # → /usr/bin/pi  v0.80.10
```

- Rust MSRV 真实为 **1.88**（注意 `Cargo.toml:27` 写的是 1.85，与 `rust-toolchain.toml`/CI 不一致，属已知文档漂移）。
- pi 真身：`/usr/lib/node_modules/@earendil-works/pi-coding-agent/dist/cli.js`
  - sha256：`af302f231437eaf6f37691bce4b34234fcb626bcb5eb3910d4fc3f6519bf78ca`

## 2. 构建 ✅

```bash
cd /home/aurobear/Workspace/aletheon
CARGO_TARGET_DIR="$PWD/target" bash scripts/cargo-agent.sh build -p aletheon --release
```

- 坑：`scripts/cargo-agent.sh` 默认把产物放 `~/.cache/aletheon-cargo/target`，**必须显式 `CARGO_TARGET_DIR="$PWD/target"`**
  才能落到部署文档引用的 `target/release/aletheon`（`scripts/cargo-agent.sh:5`）。
- 首次全量约 5m36s；增量重建（executive+aletheon）约 1m53s。

## 3. Provider 打通（Leju deepseek）✅

- 复用 aurb `config/config.yaml` 里的 `lejurobot_deepseek` key（提取时不打印明文）。
- **关键坑**：`base_url` **不能带 `/v1`**。openai transport 自己拼 `/v1/chat/completions`
  （`crates/cognit/src/impl/llm/openai_provider.rs:406`）。带 `/v1` → `.../v1/v1/...` → 404。
- 正确值：`base_url = "https://aiapi.lejurobot.com"`，`transport = "openai"`，`model = "deepseek/deepseek-v4-pro"`。
- 密钥注入约定：api_key 为空时回退环境变量 `<NAME大写>_API_KEY`
  （`crates/cognit/src/impl/provider_registry.rs:187`）→ provider 名 `leju` → `LEJU_API_KEY`。

## 4. systemd 部署（core + 用户 daemon）✅

```bash
# 安装（--no-enable 以便先写密钥再启用，避免空 provider.env 时 restart core）
sudo ALETHEON_BINARY="$PWD/target/release/aletheon" \
     ALETHEON_CONFIG="$PWD/config/production.toml.example" \
     bash scripts/install-systemd.sh --no-enable

# 写入 core 的 Leju 密钥（不打印明文）
python3 - <<'PY' | sudo tee /etc/aletheon/credentials/provider.env >/dev/null
# 从 aurb config.yaml 提取 lejurobot_deepseek.api_key，输出 "LEJU_API_KEY=<key>"
PY
sudo chown aletheon:aletheon /etc/aletheon/credentials/provider.env
sudo chmod 600 /etc/aletheon/credentials/provider.env

sudo usermod -aG aletheon aurobear          # 允许用户访问 core.sock（0660 aletheon 组）
sudo systemctl enable --now aletheon-core.service
```

- `install-systemd.sh` 会：建 `aletheon` 系统用户、装 `/usr/bin/aletheon`、装单元、跑 verify、
  （默认）启用备份 timer——用 `--no-enable` 跳过后手动只启用 core+socket。
- core 实测：`sg aletheon -c "ALETHEON_CORE_SOCKET=/run/aletheon/core.sock aletheon exec -p '...'"`
  → **返回真实回答，EXIT=0**。
- `aletheon exec` 不自足：推理走 core socket（`crates/executive/src/service/exec_session.rs:96-99`，
  可用 `ALETHEON_CORE_SOCKET` 覆盖，默认 `/run/aletheon/core.sock`）。

## 5. Pi 集成架构与点火（0 代码改动，纯配置）✅（注册成功）

**架构结论**（回答"pi 集成是不是一套系统 / aletheon 是否不完整"）：

- pi 集成是**完整、精细的子系统**，不是骨架：
  - 运行时：`crates/executive/src/impl/runtime/{pi.rs, pi_rpc.rs, pi_protocol.rs, worktree_recovery.rs}`
  - 生命周期：`crates/executive/src/service/agent_control/{admission,execution,settlement,memory,recovery,...}`
  - 启用后注册 `pi-coder` + `pi-rpc` 两个运行时（`request.rs:955-998`），**失败即关闭**
    （需 `pi_runtime.enabled` + worktree 恢复通过 + bubblewrap 命名空间探测通过）。
  - `approved_apply` = `ApplyCoordinator` 复查 pi 的 git diff（`services.rs:416`）。
- **但在 `666fcd6` 是"没点火"**：默认关、未配 executable、profiles 未装、仅用户 daemon 可用、
  agent_spawn 需**显式** `runtime="pi-rpc"`（`agent_control.rs:62-83` required: profile/runtime/task/budget，无隐式默认）。
- pi 是**独立智能体**：aletheon 发 `PiRpcCommand::Prompt`，pi 自跑循环、自调 LLM
  （`pi_protocol.rs:211-268`）。pi 用 `--provider/--model/--api-key`(env) 选模型；
  `--offline` 只禁"启动时网络操作"，**不禁 LLM**。
- `validate_fixed_args`（`pi.rs:336`）：必须含 `--mode json` + 全部隔离旗标；禁止 `--api-key`（key 走 env）。

**已写入的点火配置**：

- `~/.aletheon/config.toml`（用户 daemon 读取；含 provider + `[pi_runtime]`）：
  - `executable="/usr/bin/pi"`，`executable_sha256="af302f23…"`，`worktree_base="~/.aletheon/worktrees"`
  - `require_namespace_isolation=true`，`allowed_paths=["."]`，`forbidden_paths=[".git",".env",".aletheon"]`
  - `fixed_args=["--mode","json","--no-session","--no-context-files","--no-extensions","--no-skills",
    "--no-prompt-templates","--no-themes","--no-approve","--offline","--provider","openai","--model","deepseek/deepseek-v4-pro"]`
- `~/.config/aletheon/daemon.env`：`OPENAI_API_KEY=<leju>`，`OPENAI_BASE_URL=https://aiapi.lejurobot.com/v1`
- systemd 用户 drop-in `~/.config/systemd/user/aletheon.service.d/10-env.conf`：`EnvironmentFile=-%h/.config/aletheon/daemon.env`
- profiles 装到 `~/.local/state/aletheon/agents/`（admin/code/fs/net/safe-agent，共 5 个）

**daemon 启动日志确认**（`journalctl --user -u aletheon.service`）：
- `LLM provider initialized provider=deepseek/deepseek-v4-pro`
- `Pi coding runtime registered runtime_id=pi-coder`
- **`Pi resident RPC runtime registered runtime_id=pi-rpc`** ← pi 子系统上线
- `Registered compatibility AgentTool control client agents=5`

**踩过的配置错误（已修）**：
- 首次漏 `allowed_paths` → daemon 拒启：`Pi runtime allowed path scope must not be empty` → 加 `allowed_paths=["."]`。

## 6. 修复 1 个真实 daemon bug（首轮上下文硬失败）✅

**现象**：`-m`、无头 TUI、真实 TUI 均报
`Error: context source failed: conscious workspace has not observed a turn`。

**根因**：
- `context_assembler` 对 conscious space 做只读查找，缺失即 `?` 硬失败
  （`crates/executive/src/service/context_assembler.rs:103-107`）。
- space 由 `observe_turn`（`turn_pipeline.rs:420`，key=`sess_id`）懒创建，而查找用 `thread_id`
  （`context_assembler.rs:105`）；全新部署无历史 durable state → 首轮必失败。
- （用户之前能用 TUI，是因为其 daemon 有历史 turn，space 已从 durable state 恢复。）

**修复**（本机已改并重建部署）：让 conscious space 缺失时**优雅降级为 `None`**，不阻塞 turn
（下游 `assemble()` 本就把 `conscious` 当 `Option` 处理，`context_assembler.rs:130-144`）。

```rust
// crates/executive/src/service/context_assembler.rs（源码 ~103）
let conscious = match self.conscious.latest_context(&AgoraSpaceId(request.context.thread_id.0.clone())).await {
    Ok(projection) => { projection.validate().map_err(...)?; Some(projection) }
    Err(_) => None,   // 首轮/无 durable state：非致命，降级为无 conscious 上下文
};
```

重建部署后该错误消失 ✓（新二进制 sha `446ff62a…`）。

---

## ⚠️ 当前唯一卡点：用户 daemon 缺 aletheon 组

真实 TUI 现报 **`inference provider failed: Permission denied (os error 13)`**。

**已确证根因**：
- daemon(PID 61710) 与父 `user@1000.service`(PID 1033) 的进程组均为 `4 24 27 30 46 100 114 1000`，
  **无 `aletheon`(984)** —— 因为用户管理器在 `usermod -aG aletheon aurobear` **之前**已启动。
- core.sock = `srw-rw---- aletheon:aletheon`，daemon 无组 → 推理连接 EACCES。
- `aletheon` 组(984) 已含 aurobear（`/etc/group`），只是运行中的管理器未加载。

**解法（等价"重登激活组"，不杀我的会话）**：

```bash
sudo systemctl restart user@1000.service   # 刷新用户管理器组，session-3.scope(bash/tmux) 不受影响
# 或：用户手动重登一次 SSH/终端
```

刷新后验证：`grep ^Groups /proc/$(systemctl --user show -p MainPID --value user@1000.service)/status` 应含 984。

---

## 待办（Remaining）

1. **刷新组** → 真实 TUI(tmux) 重测首轮（应通）。测试走 `aletheon-tester` skill / tmux 真实 TUI，
   **不要用 `aletheon -m`**（单消息路径 conscious workspace 未初始化）。
2. **验证 pi 闭环**：TUI 发任务，令主 agent `agent_spawn(runtime="pi-rpc", profile="code-agent", task=…, budget=…)`
   → pi 在 bubblewrap worktree 干活 → 主 agent 复查 diff → 结果落 Mnemosyne。
   - 待观察经验点：pi 沙箱内 node/node_modules 绑定、Leju 模型 id 兼容性（实测即改）。
3. **gbrain 记忆投影**：`~/.aletheon/config.toml` 启用 `[memory.gbrain]` → `http://127.0.0.1:3131/mcp`，
   `bearer_token_env="GBRAIN_TOKEN"`（复用 aurb 的 write token），加 `[[mcp_servers]]`。先 pi 通再叠加。
4. **M4 定时闭环**：`systemctl --user` timer（克隆 backup.timer 思路）→ 包装脚本 → 向用户 daemon 发固定任务
   → pi 执行 + 总结记忆 + gbrain 投影。

## 关键路径与命令速查

| 项 | 值 |
|---|---|
| 二进制 | `/usr/bin/aletheon`（当前 sha `446ff62a…`） |
| core 单元 | `aletheon-core.service`（系统），socket `/run/aletheon/core.sock` |
| 用户 daemon | `aletheon.service`/`aletheon.socket`，socket `/run/user/1000/aletheon/aletheon.sock` |
| core 密钥 | `/etc/aletheon/credentials/provider.env`（`LEJU_API_KEY`） |
| 用户配置 | `~/.aletheon/config.toml`（provider + `[pi_runtime]`） |
| 用户 daemon 密钥 | `~/.config/aletheon/daemon.env`（pi 的 `OPENAI_*`） |
| profiles | `~/.local/state/aletheon/agents/*.md` |
| 用户日志 | `journalctl --user -u aletheon.service` |
| 真实 TUI | `XDG_RUNTIME_DIR=/run/user/1000 tmux new -d -s t 'exec /usr/bin/aletheon'` |

## 修改的仓库文件（未提交）

- `crates/executive/src/service/context_assembler.rs` — conscious space 缺失优雅降级（首轮修复）。
