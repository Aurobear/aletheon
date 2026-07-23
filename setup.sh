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
else
    BIN_DIR="$HOME/.local/bin"
    CFG_DIR="$HOME/.config/aletheon"
    SYS_SVC="$HOME/.config/systemd/user/aletheon.service"
    SYS_SCOPE="user"
    SOCKET_PATH="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/aletheon/aletheon.sock"
    DATA_DIR="$HOME/.local/share/aletheon"
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

# ── 2.5 Pre-install cleanup ──────────────────────────────────────────────

log "Stopping existing services and cleaning up..."

# Stop systemd service if running
if [[ "$MODE" == "system" ]]; then
    if systemctl is-active --quiet aletheon.service 2>/dev/null; then
        sudo systemctl stop aletheon.service
        log "Stopped aletheon.service"
    fi
else
    if systemctl --user is-active --quiet aletheon.service 2>/dev/null; then
        systemctl --user stop aletheon.service
        log "Stopped user aletheon.service"
    fi
fi

# Stop via systemd (the proper way)
if systemctl is-active --quiet aletheon.service 2>/dev/null; then
    echo "Stopping aletheon.service..."
    systemctl stop aletheon.service
fi
# Fallback: use pidfile if running outside systemd
if [[ -f /run/aletheon/aletheon.pid ]]; then
    echo "Killing daemon via pidfile..."
    kill "$(cat /run/aletheon/aletheon.pid)" 2>/dev/null || true
fi
# Clean stale socket
rm -f /run/aletheon/aletheon.sock

log "Pre-install cleanup complete"

# ── 3. Build ─────────────────────────────────────────────────────────────

BINARY_PATH="target/release/aletheon"
SKIP_BUILD=false
if [[ "${1:-}" == "--no-build" ]]; then
    SKIP_BUILD=true; shift
elif [[ "${1:-}" == "--skip-build" ]]; then
    SKIP_BUILD=true; shift
fi

if $SKIP_BUILD; then
    if [[ ! -f "$BINARY_PATH" ]]; then
        die "--no-build specified but binary not found at $BINARY_PATH"
    fi
    log "Skipping build (--no-build)"
else
    # Auto-detect: skip if binary is newer than all source files
    if [[ -f "$BINARY_PATH" ]]; then
        NEWEST_SRC=$(find crates/ Cargo.toml Cargo.lock -name "*.rs" -o -name "*.toml" -o -name "*.lock" 2>/dev/null | xargs stat -c '%Y' 2>/dev/null | sort -rn | head -1)
        BIN_MTIME=$(stat -c '%Y' "$BINARY_PATH" 2>/dev/null)
        if [[ -n "$NEWEST_SRC" && -n "$BIN_MTIME" ]] && [[ "$BIN_MTIME" -ge "$NEWEST_SRC" ]]; then
            log "Binary is up-to-date (skipping build, use --rebuild to force)"
            SKIP_BUILD=true
        fi
    fi
fi

if [[ "${1:-}" == "--rebuild" ]]; then
    SKIP_BUILD=false; shift
fi

if ! $SKIP_BUILD; then
    # Clean stale build artifacts from old binaries (removed [[bin]] entries)
    for stale_bin in aletheond aletheon-exec aletheon-systemd aletheon-container aletheon-body-cli; do
        rm -f "target/release/$stale_bin" "target/debug/$stale_bin" "target/release/$stale_bin.d" "target/debug/$stale_bin.d"
    done
    log "Cleaned stale build artifacts"

    # When running via sudo, cargo must run as the original user.
    # Otherwise $HOME=/root and cargo has no registry cache → full rebuild every time.
    if [[ -n "${SUDO_USER:-}" ]] && [[ "$(whoami)" == "root" ]]; then
        log "Running as root (sudo) — building as $SUDO_USER to reuse cargo cache"
        # ensure target/ is writable by the original user
        chown -R "$SUDO_USER:$SUDO_USER" target/ 2>/dev/null || true
        log "Building release binary through scripts/cargo-agent.sh..."
        sudo -u "$SUDO_USER" env CARGO_TARGET_DIR="$PWD/target" \
            bash scripts/cargo-agent.sh build -p aletheon --release 2>&1
    else
        log "Building release binary through scripts/cargo-agent.sh..."
        CARGO_TARGET_DIR="$PWD/target" \
            bash scripts/cargo-agent.sh build -p aletheon --release 2>&1
    fi

    if [[ ! -f "$BINARY_PATH" ]]; then
        die "Build failed — binary not found at $BINARY_PATH"
    fi
    log "Build complete: $BINARY_PATH"
fi

# ── 4. Install binary ────────────────────────────────────────────────────

mkdir -p "$BIN_DIR"
$USE_SUDO cp "$BINARY_PATH" "$BIN_DIR/aletheon"
$USE_SUDO chmod +x "$BIN_DIR/aletheon"
log "Binary installed: $BIN_DIR/aletheon"

# Clean up stale old binaries from previous installs (aletheond, aletheon-exec, etc.)
for stale in aletheond aletheon-exec aletheon-systemd aletheon-container; do
    if [[ -f "$BIN_DIR/$stale" ]]; then
        $USE_SUDO rm -f "$BIN_DIR/$stale"
        log "Removed stale binary: $BIN_DIR/$stale"
    fi
done
# Also clean common alternative locations
for stale in aletheond aletheon-exec aletheon-systemd aletheon-container aletheon; do
    for dir in /usr/local/bin "$HOME/.local/bin"; do
        if [[ "$dir/$stale" != "$BIN_DIR/$stale" ]] && [[ -f "$dir/$stale" ]]; then
            rm -f "$dir/$stale" 2>/dev/null || sudo rm -f "$dir/$stale" 2>/dev/null || true
            log "Removed stale binary: $dir/$stale"
        fi
    done
done

# ── 5. Config ────────────────────────────────────────────────────────────

setup_config() {
    $USE_SUDO mkdir -p "$CFG_DIR" "$DATA_DIR" "$(dirname "$SOCKET_PATH")"
    if [[ "$MODE" == "system" ]]; then
        $USE_SUDO chown -R "$(whoami):$(whoami)" "$DATA_DIR" 2>/dev/null || true
    else
        mkdir -p "$(dirname "$SOCKET_PATH")"
    fi

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
max_iterations = 0
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
        # Add ALETHEON_SOCKET if missing (for MCP monitor bridge)
        if ! grep -q "^ALETHEON_SOCKET=" "$env"; then
            echo "ALETHEON_SOCKET=$SOCKET_PATH" | $USE_SUDO tee -a "$env" > /dev/null
            log "Added ALETHEON_SOCKET=$SOCKET_PATH to existing env file"
        fi
    else
        log "Creating placeholder .env..."
        $USE_SUDO tee "$env" > /dev/null <<ENV
# Aletheon provider API keys
# Each provider reads <NAME>_API_KEY. Uncomment and fill in your keys.

LEJU_API_KEY=
# MIMO_API_KEY=tp-...
# DEEPSEEK_API_KEY=sk-...
# OPENAI_API_KEY=sk-...
# ANTHROPIC_API_KEY=sk-ant-...
# OLLAMA_API_KEY=ollama

# Aletheon daemon socket path (auto-detected by setup.sh)
ALETHEON_SOCKET=$SOCKET_PATH
ENV
        $USE_SUDO chmod 600 "$env"
        log "Env file written to $env"
    fi
}

setup_config

# ── 5.5. MCP Monitor Bridge ────────────────────────────────────────────────

MONITOR_SRC="$SCRIPT_DIR/tools/aletheon-monitor"
MONITOR_DST="$DATA_DIR/monitor"

setup_monitor() {
    if [[ ! -d "$MONITOR_SRC" ]]; then
        warn "Monitor source not found at $MONITOR_SRC — skipping MCP monitor install"
        return 0
    fi

    log "Installing MCP monitor bridge..."

    # Copy monitor Python package to data dir
    $USE_SUDO mkdir -p "$MONITOR_DST"
    $USE_SUDO cp -a "$MONITOR_SRC"/* "$MONITOR_DST/"
    $USE_SUDO chmod +x "$MONITOR_DST/run.py"

    # Create wrapper script at $BIN_DIR/aletheon-monitor
    # This wrapper sources the env file so ALETHEON_SOCKET is always set
    local wrapper="$BIN_DIR/aletheon-monitor"
    $USE_SUDO tee "$wrapper" > /dev/null <<WRAPPER
#!/usr/bin/env bash
# Aletheon Monitor MCP Server — auto-generated by setup.sh
# Sources the aletheon env file for socket path, then launches the monitor.

if [[ -f /etc/aletheon/.env ]]; then
    set -a; source /etc/aletheon/.env; set +a
elif [[ -f "\$HOME/.config/aletheon/.env" ]]; then
    set -a; source "\$HOME/.config/aletheon/.env"; set +a
fi

exec python3 "$MONITOR_DST/run.py" "\$@"
WRAPPER
    $USE_SUDO chmod +x "$wrapper"
    log "Monitor wrapper installed: $wrapper"
    log "Monitor files: $MONITOR_DST"
}

setup_monitor

# ── 6. Systemd service ───────────────────────────────────────────────────

setup_systemd() {
    if [[ "$MODE" == "system" ]]; then
        # Create aletheon system user and group if they don't exist.
        if ! id -u aletheon &>/dev/null; then
            echo "Creating aletheon system user..."
            useradd -r -s /usr/sbin/nologin -d /var/lib/aletheon aletheon
        fi
        # Add the real user (SUDO_USER or current user) to aletheon group.
        local real_user="${SUDO_USER:-$USER}"
        if [[ "$real_user" != "root" ]] && [[ "$real_user" != "aletheon" ]]; then
            if usermod -a -G aletheon "$real_user"; then
                log "Added $real_user to aletheon group"
                warn "The current login session may not have the aletheon group active yet."
                echo "  Verify in your login shell: id -nG | grep -w aletheon"
                echo "  Activate permanently: log out and log back in"
                echo "  Activate a subshell now: newgrp aletheon"
                echo "  One-off TUI command: sg aletheon -c 'aletheon'"
            else
                warn "Failed to add $real_user to the aletheon group"
            fi
        fi

        # Ensure /etc/aletheon/.env exists for API keys.
        if [[ ! -f /etc/aletheon/.env ]]; then
            local env_src="${CFG_DIR}/.env"
            if [[ -f "$env_src" ]]; then
                cp "$env_src" /etc/aletheon/.env
            elif [[ -f "$HOME/.aletheon/.env" ]]; then
                cp "$HOME/.aletheon/.env" /etc/aletheon/.env
            else
                touch /etc/aletheon/.env
            fi
            chmod 600 /etc/aletheon/.env
            chown aletheon:aletheon /etc/aletheon/.env
            echo "Created /etc/aletheon/.env owned by aletheon:aletheon"
        fi

        # Ensure socket/data directories are owned by aletheon.
        $USE_SUDO chown -R aletheon:aletheon "$DATA_DIR" "$(dirname "$SOCKET_PATH")" 2>/dev/null || true

        $USE_SUDO cp "$SCRIPT_DIR/config/aletheon.service" "$SYS_SVC"
        $USE_SUDO systemctl daemon-reload
        $USE_SUDO systemctl enable aletheon.service
        log "Systemd service installed: $SYS_SVC"
        # Auto-start daemon now
        $USE_SUDO systemctl start aletheon.service
        log "Daemon started"
        echo ""
        echo "  Commands:"
        echo "    sudo systemctl start aletheon    # start daemon"
        echo "    sudo systemctl status aletheon   # check status"
        echo "    journalctl -u aletheon -f        # follow logs"
    else
        mkdir -p "$(dirname "$SYS_SVC")"
        sed "s|ExecStart=%h/.local/bin/aletheon daemon|ExecStart=$BIN_DIR/aletheon daemon|" \
            "$SCRIPT_DIR/config/aletheon.user.service" > "$SYS_SVC"
        cp "$SCRIPT_DIR/config/aletheon.user.socket" \
            "$(dirname "$SYS_SVC")/aletheon.socket"
        systemctl --user daemon-reload
        systemctl --user enable --now aletheon.socket
        log "Systemd user socket installed and enabled: $(dirname "$SYS_SVC")/aletheon.socket"
        echo ""
        echo "  Commands:"
        echo "    systemctl --user start aletheon.socket  # accept client connections"
        echo "    systemctl --user status aletheon.socket # check socket status"
        echo "    systemctl --user status aletheon.service # check runtime status"
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
echo "  Monitor:   $BIN_DIR/aletheon-monitor"
echo ""
if [[ "$MODE" == "system" ]]; then
    echo "  Quick start:"
    echo "    sudo systemctl start aletheon"
    echo "    aletheon                     # launch TUI"
    echo "    aletheon daemon              # foreground debug"
    echo "    aletheon exec -p 'hello'     # non-interactive run"
else
    echo "  Quick start:"
    echo "    systemctl --user start aletheon.socket"
    echo "    aletheon                     # launch TUI"
    echo "    aletheon daemon              # foreground debug"
    echo "    aletheon exec -p 'hello'     # non-interactive run"
fi
echo ""
