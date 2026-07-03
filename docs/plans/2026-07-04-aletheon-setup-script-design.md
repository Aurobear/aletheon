# Aletheon 部署与脚本管理系统设计

**Date:** 2026-07-04
**Reference:** Codex CLI (`references/cli-agent/codex/`) — `justfile` + `scripts/install/` 模式

## 1. 目标

提供一键部署能力，简化从零到可用的完整流程。统一当前 5 个分散二进制为 1 个入口。

## 2. 最终产物

```
aletheon/
├── justfile              # 开发任务中枢
├── setup.sh              # 一键环境搭建 + 部署
├── Cargo.toml            # workspace（新增 aletheon crate 合并入口）
├── crates/
│   ├── aletheon/         # [NEW] 统一二进制入口 crate
│   │   └── src/main.rs   #   子命令分发（daemon/exec/TUI/version）
│   ├── runtime/          # 核心运行时（保留，去掉多余的 [[bin]]）
│   └── interact/         # TUI 客户端（保留，去掉 [[bin]]）
└── config/
    └── aletheon.service  # systemd unit 模板
```

## 3. `aletheon` 统一二进制

### 3.1 命令结构

```
aletheon                        # TUI 客户端（默认）
aletheon -m <message>           # 单条消息发给 daemon
aletheon exec <prompt>          # 非交互执行
    --json                      #   可选 JSON 输出
aletheon daemon                 # 启动 daemon（自动感知运行环境）
aletheon version                # 版本号 + git commit
```

### 3.2 Daemon 自动感知

```
aletheon daemon 启动时检测：
  $NOTIFY_SOCKET 存在 → SystemdHost（sd_notify + watchdog）
  $CONTAINER 环境变量   → ContainerHost（docker/podman CLI）
  都不是               → DaemonHost（unix socket，foreground）
```

### 3.3 消除的二进制

| 旧二进制 | 替代 |
|----------|------|
| `aletheond` | `aletheon daemon` |
| `aletheon-systemd` | `aletheon daemon`（自动检测 systemd） |
| `aletheon-container` | `aletheon daemon`（自动检测 container） |
| `aletheon-exec` | `aletheon exec` |

所有原 `[[bin]]` 定义从 `crates/runtime/Cargo.toml` 和 `crates/interact/Cargo.toml` 移除，统一到 `crates/aletheon/Cargo.toml`。

## 4. `justfile` 开发任务

对标 Codex 的 `justfile`。纯声明式，不需要 `aletheon` 二进制就能运行。

```justfile
# 默认目标
default:
    @just --list

# ── 构建 ──
build:
    cargo build --release

build-dev:
    cargo build

# ── 验证 ──
test:
    cargo test --workspace --all-targets

lint:
    cargo clippy --workspace --all-targets -- -D warnings

fmt:
    cargo fmt --all -- --check

fix:
    cargo fmt --all
    cargo clippy --workspace --all-targets --fix --allow-dirty

doc:
    RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps

# 全量验证（CI 同等）
check: fmt test lint doc
    @echo "=== ALL CHECKS PASSED ==="

# ── 部署 ──
install: build
    @echo "Run: sudo ./setup.sh"
    sudo bash setup.sh

# ── 清理 ──
clean:
    cargo clean
```

## 5. `setup.sh` 一键部署

### 5.1 安装路径

| 组件 | system（默认） | user（`--user`） |
|------|--------------|-----------------|
| Binary | `/usr/bin/aletheon` | `$HOME/.local/bin/aletheon` |
| Config | `/etc/aletheon/config.toml` | `$HOME/.config/aletheon/config.toml` |
| Env | `/etc/aletheon/.env` | `$HOME/.config/aletheon/.env` |
| Socket | `/run/aletheon/aletheon.sock` | `$XDG_RUNTIME_DIR/aletheon/aletheon.sock` |
| Data | `/var/lib/aletheon/` | `$HOME/.local/share/aletheon/` |
| systemd | `/etc/systemd/system/aletheon.service` | `$HOME/.config/systemd/user/aletheon.service` |

### 5.2 流程

```
setup.sh [--user]

1. check_rust     → 检测 rustc，没有则安装 rustup
2. check_deps     → 检测系统依赖（sqlite, bubblewrap），按包管理器安装
3. cargo build    → cargo build --release
4. install_binary → cp target/release/aletheon → /usr/bin/（或 ~/.local/bin/）
5. setup_config   → 写 /etc/aletheon/config.toml + .env（不覆盖已存在的）
6. setup_systemd  → 写 unit 文件 → systemctl daemon-reload → systemctl enable
```

### 5.3 systemd Unit

```ini
[Unit]
Description=Aletheon AI Agent Daemon
After=network.target

[Service]
Type=simple
ExecStart=/usr/bin/aletheon daemon
Restart=on-failure
RestartSec=5
Environment=RUST_LOG=info
RuntimeDirectory=aletheon
RuntimeDirectoryMode=0755
StateDirectory=aletheon
StateDirectoryMode=0755
ReadWritePaths=/etc/aletheon

[Install]
WantedBy=multi-user.target
```

### 5.4 配置模板

`setup.sh` 生成的 `config.toml` 包含 6 个 provider（leju/mimo/deepseek/openai/anthropic/ollama），API key 留空在 `.env`。

### 5.5 User 模式差异

`--user` 参数：
- systemd 用 `systemctl --user`，unit 写到 `~/.config/systemd/user/`
- `WantedBy=default.target`（非 `multi-user.target`）
- 不需要 `sudo`

## 6. 日常使用

```bash
# 首次部署
git clone <repo> && cd aletheon
./setup.sh                          # 系统级，需要 sudo
./setup.sh --user                   # 用户级

# 日常开发
just build                          # 编译
just check                          # 全量验证
just fix                            # 自动格式化 + clippy fix

# 日常运行
systemctl start aletheon            # 启动 daemon
systemctl status aletheon           # 查看状态
journalctl -u aletheon -f           # 跟踪日志
aletheon                            # 启动 TUI

# 更新
git pull && just build && systemctl restart aletheon
```

## 7. 实现范围

| 任务 | 说明 |
|------|------|
| 新建 `crates/aletheon/` | 统一入口 crate，子命令分发 |
| 合并 5 个 [[bin]] | runtime/interact 去掉独立 binary，所有入口走 aletheon |
| `justfile` | 开发任务中枢 |
| `setup.sh` | 重写，system/user 双模式 |
| `config/aletheon.service` | systemd unit 模板 |
| 清理旧文件 | `aletheon-systemd.rs`、`aletheon-container.rs`、`aletheond.rs` 移除 |

## 8. 非目标

- 不做 GitHub Release 自动发布（未来）
- 不做版本化多版本共存（未来）
- 不改变 JSON-RPC 协议或 TUI 行为
- 不做 Docker/Podman 镜像构建（保留 ContainerHost 代码但不扩展）
