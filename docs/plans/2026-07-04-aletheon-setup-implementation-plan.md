# Aletheon Setup & Unified Entry Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Merge 5 binaries into 1 `aletheon` entry point + `justfile` for dev tasks + `setup.sh` for one-click deploy.

**Architecture:** New `crates/aletheon/` crate as thin dispatcher using clap subcommands. All existing host logic stays in `runtime` crate. TUI client stays in `interact` crate. Shim binary files removed.

**Tech Stack:** Rust 1.85, clap 4 derive, just (task runner), bash 4+ (setup script), systemd 255+

---

## File Map

| Action | Path | Purpose |
|--------|------|---------|
| CREATE | `crates/aletheon/Cargo.toml` | New crate manifest |
| CREATE | `crates/aletheon/src/main.rs` | Unified CLI entry point |
| MODIFY | `Cargo.toml` | Add `crates/aletheon` to workspace members |
| MODIFY | `crates/runtime/Cargo.toml` | Remove 4 `[[bin]]` entries |
| MODIFY | `crates/interact/Cargo.toml` | Remove 1 `[[bin]]` entry |
| DELETE | `crates/runtime/src/bin/aletheond.rs` | Absorbed into aletheon |
| DELETE | `crates/runtime/src/bin/aletheon-systemd.rs` | Absorbed into aletheon |
| DELETE | `crates/runtime/src/bin/aletheon-container.rs` | Absorbed into aletheon |
| DELETE | `crates/runtime/src/bin/aletheon-exec.rs` | Absorbed into aletheon |
| DELETE | `crates/interact/src/bin/aletheon.rs` | Absorbed into aletheon |
| CREATE | `justfile` | Dev task runner |
| MODIFY | `setup.sh` | Rewrite for one-click deploy |
| CREATE | `config/aletheon.service` | Systemd unit template |

---

### Task 1: Create `crates/aletheon/` unified entry crate

**Files:**
- Create: `crates/aletheon/Cargo.toml`
- Create: `crates/aletheon/src/main.rs`
- Modify: `Cargo.toml`

- [ ] **Step 1: Create crate manifest**

File: `crates/aletheon/Cargo.toml`

```toml
[package]
name = "aletheon"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
description = "Aletheon — unified AI agent CLI (daemon, exec, TUI)"

[[bin]]
name = "aletheon"
path = "src/main.rs"

[dependencies]
runtime = { path = "../runtime" }
interact = { path = "../interact" }
base = { path = "../base" }
cognit = { path = "../cognit" }
anyhow = { workspace = true }
tokio = { workspace = true }
clap = { version = "4", features = ["derive"] }
tracing = { workspace = true }
tracing-subscriber = { workspace = true }
```

- [ ] **Step 2: Add to workspace**

In `Cargo.toml` (workspace root), add `"crates/aletheon"` to members:

```toml
members = [
    "crates/base",
    "crates/cognit",
    "crates/corpus",
    "crates/dasein",
    "crates/interact",
    "crates/memory",
    "crates/metacog",
    "crates/runtime",
    "crates/aletheon",
    "examples/basic-agent",
    "examples/self-evolution-loop",
]
```

- [ ] **Step 3: Create main.rs — unified CLI**

File: `crates/aletheon/src/main.rs`

```rust
//! aletheon — unified entry point for Aletheon AI agent.
//!
//! Subcommands:
//!   (none)       TUI client (auto-starts daemon if not running)
//!   daemon       Start daemon (auto-detects systemd/container/foreground)
//!   exec         Non-interactive execution
//!   -m <msg>     Send single message to daemon
//!   version      Print version + git commit

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "aletheon", about = "AI agent with sandbox, multi-agent, IPC")]
#[command(version = concat!("0.1.0 (", env!("VERGEN_GIT_SHA", "unknown"), ")"))]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Send a single message to the daemon
    #[arg(short = 'm', long = "message", value_name = "MSG")]
    message: Option<String>,

    /// Socket path (default: /run/aletheon/aletheon.sock)
    #[arg(short, long, default_value = "/run/aletheon/aletheon.sock")]
    socket: PathBuf,
}

#[derive(Subcommand)]
enum Commands {
    /// Start daemon (auto-detects systemd/container/foreground)
    Daemon {
        /// Path to config file
        #[arg(short, long)]
        config: Option<PathBuf>,
        /// Path to .env file
        #[arg(long)]
        env: Option<PathBuf>,
        /// Socket path (overrides parent --socket)
        #[arg(short, long)]
        socket: Option<PathBuf>,
        /// Force container mode (docker/podman)
        #[arg(long)]
        container: Option<String>,
        /// Container image name
        #[arg(long, default_value = "aletheon:latest")]
        image: String,
    },
    /// Non-interactive execution
    Exec {
        /// The prompt/task to execute
        #[arg(short, long)]
        prompt: String,
        /// Model spec
        #[arg(short, long, default_value = "")]
        model: String,
        /// Maximum agentic turns
        #[arg(short = 'n', long, default_value_t = 20)]
        max_turns: usize,
        /// Sandbox preference: auto, require, or forbid
        #[arg(long, default_value = "auto")]
        sandbox: String,
        /// Working directory for tool execution
        #[arg(short = 'd', long, default_value = ".")]
        working_dir: PathBuf,
        /// Path to config file
        #[arg(short, long)]
        config: Option<PathBuf>,
        /// Output format: text or json
        #[arg(long, default_value = "text")]
        output: String,
    },
    /// Print version
    Version,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match (&cli.command, &cli.message) {
        // Text message mode (-m flag)
        (None, Some(msg)) => interact::cli::single_message(&cli.socket, msg).await,
        (None, None) => {
            // Default: TUI client
            interact::cli::run().await
        }
        (Some(Commands::Daemon { config, env, socket, container, image }), _) => {
            init_tracing("aletheon::daemon");
            let socket_path = socket.clone().unwrap_or(cli.socket);
            let daemon_mode = detect_daemon_mode(container);

            match daemon_mode {
                DaemonMode::Systemd => {
                    let mut host = runtime::host::systemd::SystemdHost::new(
                        config.clone(), env.clone(), socket_path,
                    );
                    host.init().await?;
                    Box::new(host).serve().await
                }
                DaemonMode::Container { runtime_name } => {
                    let mut host = runtime::host::container::ContainerHost::new(
                        config.clone(), env.clone(), runtime_name, image.clone(),
                    );
                    host.init().await?;
                    Box::new(host).serve().await
                }
                DaemonMode::Foreground => {
                    let mut host = runtime::host::DaemonHost::new(
                        config.clone(), env.clone(), socket_path,
                    );
                    host.init().await?;
                    Box::new(host).serve().await
                }
            }
        }
        (Some(Commands::Exec { prompt, model, max_turns, sandbox, working_dir, config, output }), _) => {
            run_exec(prompt, model, *max_turns, sandbox, working_dir, config, output).await
        }
        (Some(Commands::Version), _) => {
            println!("aletheon 0.1.0");
            println!("git: {}", option_env!("VERGEN_GIT_SHA").unwrap_or("unknown"));
            Ok(())
        }
        _ => {
            interact::cli::run().await
        }
    }
}

enum DaemonMode {
    Systemd,
    Container { runtime_name: String },
    Foreground,
}

fn detect_daemon_mode(container_override: &Option<String>) -> DaemonMode {
    // Explicit --container flag
    if let Some(rt) = container_override {
        return DaemonMode::Container {
            runtime_name: rt.clone(),
        };
    }
    // Auto-detect systemd via $NOTIFY_SOCKET
    if std::env::var("NOTIFY_SOCKET").is_ok() {
        return DaemonMode::Systemd;
    }
    // Auto-detect container via $CONTAINER or /.dockerenv
    if std::env::var("CONTAINER").is_ok() || std::path::Path::new("/.dockerenv").exists() {
        return DaemonMode::Container {
            runtime_name: "docker".to_string(),
        };
    }
    DaemonMode::Foreground
}

fn init_tracing(target: &str) {
    if std::env::var("RUST_LOG").is_ok() {
        tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::from_default_env())
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::new(format!("{}=info", target)))
            .init();
    }
}

async fn run_exec(
    prompt: &str,
    model: &str,
    max_turns: usize,
    sandbox: &str,
    working_dir: &std::path::Path,
    config: &Option<PathBuf>,
    output: &str,
) -> Result<()> {
    use std::sync::Arc;

    init_tracing("aletheon::exec");

    // Determine config path (default: ~/.aletheon/config.toml)
    let config_path = config.clone().unwrap_or_else(|| {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".aletheon")
            .join("config.toml")
    });

    // Determine env path
    let env_path = if let Some(ref c) = config {
        c.parent().unwrap_or(std::path::Path::new(".")).join(".env")
    } else {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".aletheon")
            .join(".env")
    };

    // Load .env
    if env_path.exists() {
        runtime::host::load_dotenv(&env_path);
    }

    // Create provider registry from config
    use cognit::r#impl::provider_registry::ProviderRegistry;
    use cognit::r#impl::llm::LlmProvider;
    let registry = ProviderRegistry::new();
    let llm: Arc<dyn LlmProvider> = registry.resolve_and_create(&config_path, model)?;

    // Create tool registry
    use corpus::tools::tools::{ToolContext, ToolRegistry};
    use corpus::security::security::approval::TerminalApprovalGate;
    use corpus::security::security::runner::ToolRunnerWithGuard;
    let tool_ctx = ToolContext {
        working_dir: working_dir.to_path_buf(),
        session_id: "exec".to_string(),
        approval_gate: None,
    };
    let mut tools = ToolRegistry::new();
    corpus::tools::tools::register_default_tools(&mut tools)?;
    let guard = Arc::new(TerminalApprovalGate::new());
    let runner = Arc::new(ToolRunnerWithGuard::new(
        Arc::new(tools),
        guard,
        working_dir.to_path_buf(),
    ));

    // Run agent loop
    use base::{ContentBlock, Message, Role};
    use cognit::r#impl::llm::StopReason;

    let mut messages = vec![Message {
        role: Role::User,
        content: vec![ContentBlock::Text {
            text: prompt.to_string(),
        }],
    }];
    let mut total_in = 0u32;
    let mut total_out = 0u32;
    let mut final_response = String::new();
    let mut success = false;

    for _turn in 0..max_turns {
        let resp = llm.chat(&messages, &runner.list_tools()).await?;
        total_in += resp.usage.input_tokens;
        total_out += resp.usage.output_tokens;

        // Add assistant message
        let mut assistant_blocks = Vec::new();
        let mut tool_calls = Vec::new();
        for block in &resp.content {
            match block {
                ContentBlock::Text { text } => {
                    assistant_blocks.push(ContentBlock::Text { text: text.clone() });
                    final_response = text.clone();
                }
                ContentBlock::ToolUse { id, name, input } => {
                    tool_calls.push((id.clone(), name.clone(), input.clone()));
                }
                _ => {}
            }
        }
        if !assistant_blocks.is_empty() {
            messages.push(Message {
                role: Role::Assistant,
                content: assistant_blocks,
            });
        }

        // If no tool calls, we're done
        if tool_calls.is_empty() || resp.stop_reason == StopReason::EndTurn {
            success = true;
            break;
        }

        // Execute tools
        let mut tool_results = Vec::new();
        for (id, name, input) in &tool_calls {
            let result = runner.execute(name, input.clone()).await?;
            let result_text = if result.is_error {
                format!("Error: {}", result.content)
            } else {
                result.content.clone()
            };
            tool_results.push(ContentBlock::ToolResult {
                tool_use_id: id.clone(),
                content: result_text,
                is_error: result.is_error,
            });
        }
        messages.push(Message {
            role: Role::User,
            content: tool_results,
        });
    }

    if output == "json" {
        let result = serde_json::json!({
            "success": success,
            "response": final_response,
            "turns_used": messages.len() / 2,
            "total_input_tokens": total_in,
            "total_output_tokens": total_out,
        });
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!("{}", final_response);
    }

    if !success {
        std::process::exit(1);
    }
    Ok(())
}
```

Note: The `exec` subcommand currently embeds the exec logic directly (originally from `aletheon-exec.rs`). The `send_message` function needs a new helper in `interact::cli`. Let's handle this simply — if `-m` is given and no daemon is detected, we auto-start the daemon and send the message via the TUI client path.

Actually, looking at the current wrapper script (`setup.sh:279`), the `-m` mode just calls the `aletheon` binary with `--socket`. The `interact::cli` module already handles this. Let me simplify:

- [ ] **Step 4: Verify compilation**

Run: `cargo check -p aletheon 2>&1 | tail -5`
Expected: `Finished dev profile`

- [ ] **Step 5: Commit**

```bash
git add crates/aletheon/ Cargo.toml
git commit -m "feat: create unified aletheon entry crate with subcommands"
```

---

### Task 2: Remove old [[bin]] entries and bin files

**Files:**
- Modify: `crates/runtime/Cargo.toml`
- Modify: `crates/interact/Cargo.toml`
- Delete: `crates/runtime/src/bin/aletheond.rs`
- Delete: `crates/runtime/src/bin/aletheon-systemd.rs`
- Delete: `crates/runtime/src/bin/aletheon-container.rs`
- Delete: `crates/runtime/src/bin/aletheon-exec.rs`
- Delete: `crates/interact/src/bin/aletheon.rs`

- [ ] **Step 1: Remove [[bin]] from runtime crate**

In `crates/runtime/Cargo.toml`, remove lines 9-23 (all 4 `[[bin]]` blocks).

- [ ] **Step 2: Remove [[bin]] from interact crate**

In `crates/interact/Cargo.toml`, remove lines 9-11 (`[[bin]]` block for `aletheon`).

- [ ] **Step 3: Delete old bin files**

```bash
rm crates/runtime/src/bin/aletheond.rs
rm crates/runtime/src/bin/aletheon-systemd.rs
rm crates/runtime/src/bin/aletheon-container.rs
rm crates/runtime/src/bin/aletheon-exec.rs
rm crates/interact/src/bin/aletheon.rs
```

- [ ] **Step 4: Verify workspace still compiles**

Run: `cargo check --workspace --all-targets 2>&1 | tail -5`
Expected: `Finished dev profile`

- [ ] **Step 5: Commit**

```bash
git add crates/runtime/Cargo.toml crates/interact/Cargo.toml
git add crates/runtime/src/bin/ crates/interact/src/bin/
git commit -m "refactor: remove old separate binaries, replaced by unified aletheon crate"
```

---

### Task 3: Create `justfile`

**Files:**
- Create: `justfile`

- [ ] **Step 1: Create justfile**

File: `justfile` (repo root)

```justfile
# ── Aletheon Dev Tasks ──────────────────────────────────────────────────
# cargo 自带增量编译，只编译变更的 crate 及其下游依赖。
# 日常开发用 dev（debug，秒级），部署前用 build（release + 全验证）。

default:
    @just --list

# ── 构建 ───────────────────────────────────────────────────────────────

# 快速增量编译（debug 模式，日常开发用）
dev:
    cargo build -p aletheon

# 编译 + 测试 + lint 全部通过后才 build release
build: test lint
    cargo build -p aletheon --release

# 查看各 crate 编译耗时
timings:
    cargo build --timings

# ── 验证 ───────────────────────────────────────────────────────────────

# 运行所有测试
test:
    cargo test --workspace --all-targets

# clippy 严格模式
lint:
    cargo clippy --workspace --all-targets -- -D warnings

# 格式化检查
fmt:
    cargo fmt --all -- --check

# 自动修复格式 + clippy 建议
fix:
    cargo fmt --all
    cargo clippy --workspace --all-targets --fix --allow-dirty --allow-no-vcs

# 生成文档
doc:
    RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps

# CI 级全量验证
check: fmt test lint doc
    @echo "=== ALL CHECKS PASSED ==="

# ── 部署 ───────────────────────────────────────────────────────────────

# 编译 release + 部署到系统
install: build
    sudo bash setup.sh

# ── 清理 ───────────────────────────────────────────────────────────────

# 删除编译缓存
clean:
    cargo clean

# ── 加速（可选） ────────────────────────────────────────────────────────

# 安装 sccache 跨构建共享缓存（clean 后重编译快 50%+）
setup-sccache:
    cargo install sccache --locked
    @echo '[build]' >> .cargo/config.toml
    @echo 'rustc-wrapper = "sccache"' >> .cargo/config.toml
    @echo "sccache configured in .cargo/config.toml"
```

- [ ] **Step 2: Test just --list**

Run: `just --list 2>&1 | head -20`
Expected: Lists all targets

- [ ] **Step 3: Test just dev**

Run: `just dev 2>&1 | tail -3`
Expected: Compiles aletheon binary in debug mode

- [ ] **Step 4: Commit**

```bash
git add justfile
git commit -m "feat: add justfile for dev tasks (build, test, lint, fmt, check)"
```

---

### Task 4: Create systemd unit template

**Files:**
- Create: `config/aletheon.service`

- [ ] **Step 1: Create systemd service template**

File: `config/aletheon.service`

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

# Security hardening
NoNewPrivileges=yes
PrivateTmp=yes
ProtectHome=read-only
ReadWritePaths=%h/.local/share/aletheon

[Install]
WantedBy=multi-user.target
```

- [ ] **Step 2: Create user-scoped variant**

File: `config/aletheon.user.service`

```ini
[Unit]
Description=Aletheon AI Agent Daemon (User)
After=network.target

[Service]
Type=simple
ExecStart=%h/.local/bin/aletheon daemon
Restart=on-failure
RestartSec=5
Environment=RUST_LOG=info
RuntimeDirectory=aletheon

[Install]
WantedBy=default.target
```

- [ ] **Step 3: Commit**

```bash
git add config/aletheon.service config/aletheon.user.service
git commit -m "feat: add systemd unit templates for system and user modes"
```

---

### Task 5: Rewrite `setup.sh`

**Files:**
- Modify: `setup.sh`

- [ ] **Step 1: Rewrite setup.sh**

File: `setup.sh` (complete rewrite)

```bash
#!/usr/bin/env bash
set -euo pipefail

# Aletheon one-click setup
# Usage:
#   ./setup.sh              # System install (needs sudo)
#   ./setup.sh --user       # User install (~/.local/bin)
#
# Flow:
#   1. Check/install Rust
#   2. Check/install system deps (sqlite, bubblewrap)
#   3. Build release binary
#   4. Install binary to system path
#   5. Write config + .env (never overwrites existing)
#   6. Install + enable systemd service

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

log()    { echo -e "${GREEN}[aletheon]${NC} $*"; }
warn()   { echo -e "${YELLOW}[warn]${NC} $*"; }
die()    { echo -e "${RED}[error]${NC} $*" >&2; exit 1; }

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# ── Parse args ────────────────────────────────────────────────────────────

MODE="system"; USE_SUDO="sudo"
if [[ "${1:-}" == "--user" ]]; then
    MODE="user"; USE_SUDO=""
    shift
fi

log "Install mode: $MODE"

# ── Paths ─────────────────────────────────────────────────────────────────

if [[ "$MODE" == "system" ]]; then
    BIN_DIR="/usr/bin"
    CFG_DIR="/etc/aletheon"
    SYS_SVC="/etc/systemd/system/aletheon.service"
    SYS_SCOPE="system"
    SOCKET_PATH="/run/aletheon/aletheon.sock"
    DATA_DIR="/var/lib/aletheon"
    LOG_PREFIX="sudo"
else
    BIN_DIR="$HOME/.local/bin"
    CFG_DIR="$HOME/.config/aletheon"
    SYS_SVC="$HOME/.config/systemd/user/aletheon.service"
    SYS_SCOPE="user"
    SOCKET_PATH="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/aletheon/aletheon.sock"
    DATA_DIR="$HOME/.local/share/aletheon"
    LOG_PREFIX=""
fi

export RUST_LOG="${RUST_LOG:-info}"

# ── 1. Rust toolchain ────────────────────────────────────────────────────

check_rust() {
    if command -v rustc &>/dev/null && command -v cargo &>/dev/null; then
        log "Rust $(rustc --version) found"
        return 0
    fi
    return 1
}

install_rust() {
    log "Installing Rust via rustup..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
    source "$HOME/.cargo/env"
    log "Rust $(rustc --version) installed"
}

if ! check_rust; then
    install_rust
fi

# Ensure rustup target dir is in PATH
export PATH="$HOME/.cargo/bin:$PATH"

# ── 2. System dependencies ───────────────────────────────────────────────

install_deps() {
    local pkgs=""
    local pkg_mgr=""

    if command -v pacman &>/dev/null; then
        pkg_mgr="pacman"; pkgs="bubblewrap sqlite"
    elif command -v apt-get &>/dev/null; then
        pkg_mgr="apt"; pkgs="bubblewrap libsqlite3-dev"
    elif command -v dnf &>/dev/null; then
        pkg_mgr="dnf"; pkgs="bubblewrap sqlite-devel"
    else
        warn "Unknown package manager — install bubblewrap and sqlite manually"
        return
    fi

    local missing=()
    for p in $pkgs; do
        case $pkg_mgr in
            pacman) pacman -Qi "$p" &>/dev/null || missing+=("$p") ;;
            apt)    dpkg -s "$p" &>/dev/null    || missing+=("$p") ;;
            dnf)    rpm -q "$p" &>/dev/null     || missing+=("$p") ;;
        esac
    done

    if [[ ${#missing[@]} -eq 0 ]]; then
        log "System dependencies OK"
        return
    fi

    log "Installing missing: ${missing[*]}"
    case $pkg_mgr in
        pacman) sudo pacman -S --noconfirm "${missing[@]}" ;;
        apt)    sudo apt-get install -y "${missing[@]}" ;;
        dnf)    sudo dnf install -y "${missing[@]}" ;;
    esac
}

install_deps

# ── 3. Build ─────────────────────────────────────────────────────────────

log "Building release binary (cargo build --release)..."
cargo build -p aletheon --release 2>&1

BINARY_PATH="target/release/aletheon"
if [[ ! -f "$BINARY_PATH" ]]; then
    die "Build failed — binary not found at $BINARY_PATH"
fi
log "Build complete: $BINARY_PATH"

# ── 4. Install binary ────────────────────────────────────────────────────

mkdir -p "$BIN_DIR"
$USE_SUDO cp "$BINARY_PATH" "$BIN_DIR/aletheon"
$USE_SUDO chmod +x "$BIN_DIR/aletheon"
log "Binary installed: $BIN_DIR/aletheon"

# ── 5. Config ────────────────────────────────────────────────────────────

setup_config() {
    $USE_SUDO mkdir -p "$CFG_DIR" "$DATA_DIR"
    $USE_SUDO chown -R "$(whoami):$(whoami)" "$DATA_DIR" 2>/dev/null || true

    local cfg="$CFG_DIR/config.toml"
    local env="$CFG_DIR/.env"

    if [[ -f "$cfg" ]]; then
        log "Config exists at $cfg (not overwriting)"
    else
        log "Creating default config..."
        $USE_SUDO tee "$cfg" > /dev/null <<'TOML'
# Aletheon configuration
# API keys: set in .env (same directory) or environment variables

[agent]
default_provider = "leju"
default_model = "deepseek/deepseek-v4-pro"
max_iterations = 50
max_tokens = 100000

# ── Providers ───────────────────────────────────────────────────

[[providers]]
name = "leju"
base_url = "https://aiapi.lejurobot.com"
api_key = ""
transport = "anthropic"
models = ["deepseek/deepseek-v4-pro"]

[[providers]]
name = "mimo"
base_url = "https://token-plan-sgp.xiaomimimo.com"
api_key = ""
transport = "auto"
models = ["mimo-v2.5-pro", "mimo-v2.5-flash"]

[[providers]]
name = "deepseek"
base_url = "https://api.deepseek.com"
api_key = ""
transport = "openai"
models = ["deepseek-v4-pro", "deepseek-v4-flash"]

[[providers]]
name = "openai"
base_url = "https://api.openai.com"
api_key = ""
transport = "openai"
models = ["gpt-4o", "gpt-4o-mini"]

[[providers]]
name = "anthropic"
base_url = "https://api.anthropic.com"
api_key = ""
transport = "anthropic"
models = ["claude-sonnet-4-20250514", "claude-opus-4-20250514"]

[[providers]]
name = "ollama"
base_url = "http://localhost:11434"
api_key = ""
transport = "openai"
models = ["qwen3:8b", "llama3:8b"]

# ── Model Aliases ────────────────────────────────────────────────

[model_aliases]
pro = "leju/deepseek/deepseek-v4-pro"
flash = "mimo/mimo-v2.5-flash"
deepseek = "deepseek/deepseek-v4-pro"
local = "ollama/qwen3:8b"
TOML
        log "Config written to $cfg"
    fi

    if [[ -f "$env" ]]; then
        log "Env file exists at $env (not overwriting)"
    else
        log "Creating placeholder .env..."
        $USE_SUDO tee "$env" > /dev/null <<'ENV'
# Aletheon provider API keys
# Uncomment and fill in the keys you want to use

ANTHROPIC_API_KEY=sk-pmqmj7SvcCpSxORyFMKEhOaTPijdzfk2dPPQMwzbYAwzhcYq
# MIMO_API_KEY=tp-...
# DEEPSEEK_API_KEY=sk-...
# OPENAI_API_KEY=sk-...
ENV
        $USE_SUDO chmod 600 "$env"
        log "Env file written to $env"
    fi
}

setup_config

# ── 6. Systemd service ───────────────────────────────────────────────────

setup_systemd() {
    if [[ "$MODE" == "system" ]]; then
        $USE_SUDO cp "$SCRIPT_DIR/config/aletheon.service" "$SYS_SVC"
        $USE_SUDO systemctl daemon-reload
        $USE_SUDO systemctl enable aletheon.service
        log "Systemd service installed: $SYS_SVC"
        echo ""
        echo "  Commands:"
        echo "    sudo systemctl start aletheon    # start daemon"
        echo "    sudo systemctl status aletheon   # check status"
        echo "    journalctl -u aletheon -f        # follow logs"
    else
        mkdir -p "$(dirname "$SYS_SVC")"
        sed "s|ExecStart=/usr/bin/aletheon daemon|ExecStart=$BIN_DIR/aletheon daemon|" \
            "$SCRIPT_DIR/config/aletheon.user.service" > "$SYS_SVC"
        systemctl --user daemon-reload
        systemctl --user enable aletheon.service
        log "Systemd user service installed: $SYS_SVC"
        echo ""
        echo "  Commands:"
        echo "    systemctl --user start aletheon    # start daemon"
        echo "    systemctl --user status aletheon   # check status"
        echo "    journalctl --user -u aletheon -f   # follow logs"
    fi
}

setup_systemd

# ── Done ─────────────────────────────────────────────────────────────────

echo ""
log "Setup complete!"
echo ""
echo "  Binary:    $BIN_DIR/aletheon"
echo "  Config:    $CFG_DIR/config.toml"
echo "  Env:       $CFG_DIR/.env"
echo "  Socket:    $SOCKET_PATH"
echo "  Data:      $DATA_DIR"
echo ""
echo "  Quick start:"
echo "    sudo systemctl start aletheon   # or: systemctl --user start aletheon"
echo "    aletheon                       # launch TUI"
echo "    aletheon daemon                # foreground debug mode"
echo "    aletheon exec -p 'hello'       # single non-interactive run"
echo ""
```

- [ ] **Step 2: Make executable**

```bash
chmod +x setup.sh
```

- [ ] **Step 3: Test --help output**

Run: `bash setup.sh --help 2>&1` or just check the script parses correctly
Expected: No syntax errors (just runs normally)

- [ ] **Step 4: Commit**

```bash
git add setup.sh
git commit -m "feat: rewrite setup.sh for system/user dual-mode one-click deploy"
```

---

### Task 6: Full validation

- [ ] **Step 1: Compile and test**

```bash
just check                                    # All 5 validations
cargo build -p aletheon --release 2>&1 | tail -3
```

Expected: All pass, `aletheon` binary in `target/release/`

- [ ] **Step 2: Verify binary works**

```bash
target/release/aletheon version               # prints version
target/release/aletheon --help                # prints help with subcommands
target/release/aletheon daemon --help         # prints daemon options
target/release/aletheon exec --help           # prints exec options
```

- [ ] **Step 3: Commit final changes**

```bash
git add -A
git commit -m "feat: complete unified aletheon entry + justfile + setup.sh"
```

---

## Dependency Order

```
Task 1 (create crate) → Task 2 (remove old bins) → Task 3 (justfile) | Task 4 (systemd) | Task 5 (setup.sh) → Task 6 (validate)
```
