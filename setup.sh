#!/usr/bin/env bash
set -euo pipefail

# Aletheon setup script
# Usage:
#   ./setup.sh              Build release + install system service
#   ./setup.sh --dev        Build debug + install system service
#   ./setup.sh --install    Skip build, install already-built binaries
#   ./setup.sh --uninstall  Remove binaries, service, wrapper (keep config)
#   ./setup.sh --status     Show service status

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

log()   { echo -e "${GREEN}[aletheon]${NC} $*"; }
warn()  { echo -e "${YELLOW}[warn]${NC} $*"; }
err()   { echo -e "${RED}[error]${NC} $*" >&2; }
info()  { echo -e "${CYAN}[info]${NC} $*"; }

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

SERVICE_NAME="aletheond"
BIN_DIR="/usr/local/bin"
CONFIG_DIR="$HOME/.aletheon"
CONFIG_FILE="$CONFIG_DIR/config.toml"
ENV_FILE="$CONFIG_DIR/.env"
SOCKET_DIR="/run/${SERVICE_NAME}"
SOCKET_PATH="${SOCKET_DIR}/${SERVICE_NAME}.sock"
WRAPPER="$HOME/.local/bin/aletheon"

# ─── Parse arguments ──────────────────────────────────────────────────
ACTION="build-and-install"
PROFILE="release"
CARGO_FLAGS="--release"

for arg in "$@"; do
    case "$arg" in
        --dev)       PROFILE="debug"; CARGO_FLAGS="" ;;
        --install)   ACTION="install" ;;
        --uninstall) ACTION="uninstall" ;;
        --status)    ACTION="status" ;;
        --help|-h)
            echo "Usage: ./setup.sh [--dev|--install|--uninstall|--status]"
            echo ""
            echo "  (default)       Build release + install system service"
            echo "  --dev           Build debug + install system service"
            echo "  --install       Skip build, install already-built binaries"
            echo "  --uninstall     Remove binaries, service, wrapper (keep config)"
            echo "  --status        Show service status"
            exit 0
            ;;
        *) err "Unknown option: $arg"; exit 1 ;;
    esac
done

TARGET_DIR="target/$PROFILE"

# ─── Uninstall ────────────────────────────────────────────────────────
do_uninstall() {
    log "Uninstalling Aletheon..."

    # Stop and disable service
    if systemctl is-active --quiet "$SERVICE_NAME" 2>/dev/null; then
        sudo systemctl stop "$SERVICE_NAME"
        log "Service stopped"
    fi
    if systemctl is-enabled --quiet "$SERVICE_NAME" 2>/dev/null; then
        sudo systemctl disable "$SERVICE_NAME" 2>/dev/null
        log "Service disabled"
    fi

    # Remove service file
    local svc="/etc/systemd/system/${SERVICE_NAME}.service"
    if [[ -f "$svc" ]]; then
        sudo rm -f "$svc"
        sudo systemctl daemon-reload
        log "Service file removed"
    fi

    # Remove binaries
    for bin in aletheond aletheon aletheon-exec; do
        if [[ -f "$BIN_DIR/$bin" ]]; then
            sudo rm -f "$BIN_DIR/$bin"
            log "Removed $BIN_DIR/$bin"
        fi
    done

    # Remove wrapper
    if [[ -f "$WRAPPER" ]]; then
        rm -f "$WRAPPER"
        log "Removed wrapper: $WRAPPER"
    fi

    # Remove socket dir
    if [[ -d "$SOCKET_DIR" ]]; then
        sudo rm -rf "$SOCKET_DIR"
        log "Removed socket dir: $SOCKET_DIR"
    fi

    # Remove PID file
    rm -f /tmp/aletheon/aletheond.pid 2>/dev/null

    echo ""
    log "Uninstall complete. Config preserved at $CONFIG_DIR/"
    echo "  To also remove config: rm -rf $CONFIG_DIR"
}

# ─── Status ───────────────────────────────────────────────────────────
do_status() {
    echo ""
    # Service status
    if systemctl is-active --quiet "$SERVICE_NAME" 2>/dev/null; then
        log "Service: active"
        systemctl status "$SERVICE_NAME" --no-pager 2>/dev/null | head -15
    else
        warn "Service: inactive"
    fi

    echo ""
    # Socket
    if [[ -S "$SOCKET_PATH" ]]; then
        log "Socket: $SOCKET_PATH (listening)"
    else
        warn "Socket: $SOCKET_PATH (not found)"
    fi

    # Binaries
    echo ""
    for bin in aletheond aletheon aletheon-exec; do
        if [[ -f "$BIN_DIR/$bin" ]]; then
            local ver
            ver=$("$BIN_DIR/$bin" --version 2>/dev/null || echo "unknown")
            log "Binary: $BIN_DIR/$bin ($ver)"
        else
            warn "Binary: $BIN_DIR/$bin (not installed)"
        fi
    done

    # Config
    echo ""
    if [[ -f "$CONFIG_FILE" ]]; then
        log "Config: $CONFIG_FILE"
    else
        warn "Config: $CONFIG_FILE (not found)"
    fi
    if [[ -f "$ENV_FILE" ]]; then
        log "Env:    $ENV_FILE"
    else
        warn "Env:    $ENV_FILE (not found)"
    fi
}

# ─── Status shortcut ──────────────────────────────────────────────────
if [[ "$ACTION" == "status" ]]; then
    do_status
    exit 0
fi

# ─── Uninstall shortcut ───────────────────────────────────────────────
if [[ "$ACTION" == "uninstall" ]]; then
    do_uninstall
    exit 0
fi

# ─── 1. Rust toolchain ───────────────────────────────────────────────
check_rust() {
    if command -v rustc &>/dev/null; then
        log "Rust $(rustc --version) found"
        return 0
    fi
    return 1
}

install_rust() {
    log "Installing Rust via rustup..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source "$HOME/.cargo/env"
    log "Rust $(rustc --version) installed"
}

if ! check_rust; then
    install_rust
fi

# ─── 2. System dependencies ──────────────────────────────────────────
install_deps() {
    local pkg_mgr=""
    local pkgs=""

    if command -v pacman &>/dev/null; then
        pkg_mgr="pacman"
        pkgs="bubblewrap sqlite"
    elif command -v apt-get &>/dev/null; then
        pkg_mgr="apt"
        pkgs="bubblewrap libsqlite3-dev"
    elif command -v dnf &>/dev/null; then
        pkg_mgr="dnf"
        pkgs="bubblewrap sqlite-devel"
    else
        warn "Unknown package manager — install bubblewrap and sqlite manually"
        return
    fi

    local missing=()
    for p in $pkgs; do
        case $pkg_mgr in
            pacman) pacman -Qi "$p" &>/dev/null || missing+=("$p") ;;
            apt)    dpkg -s "$p" &>/dev/null    || missing+=("$p") ;;
            dnf)    rpm -q "$p" &>/dev/null      || missing+=("$p") ;;
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

# ─── 3. Build ────────────────────────────────────────────────────────
do_build() {
    log "Building ($PROFILE)..."
    cargo build $CARGO_FLAGS 2>&1

    if [[ "$PROFILE" == "debug" ]]; then
        log "Running tests (debug mode)..."
        cargo test 2>&1 || warn "Some tests failed (non-fatal)"
    else
        log "Skipping tests in release mode (use --dev to run tests)"
    fi

    # Verify binaries exist
    local ok=true
    for bin in aletheond aletheon aletheon-exec; do
        if [[ ! -f "$TARGET_DIR/$bin" ]]; then
            err "Binary not found: $TARGET_DIR/$bin"
            ok=false
        fi
    done
    if ! $ok; then
        err "Build failed — binaries missing"
        exit 1
    fi

    log "Build complete: $TARGET_DIR/"
}

if [[ "$ACTION" == "build-and-install" ]]; then
    do_build
fi

# ─── 4. Install binaries ─────────────────────────────────────────────
do_install_binaries() {
    log "Installing binaries to $BIN_DIR/..."
    sudo mkdir -p "$BIN_DIR"
    for bin in aletheond aletheon aletheon-exec; do
        if [[ ! -f "$TARGET_DIR/$bin" ]]; then
            err "Binary not found: $TARGET_DIR/$bin — run without --install first"
            exit 1
        fi
        sudo cp "$TARGET_DIR/$bin" "$BIN_DIR/$bin"
        sudo chmod +x "$BIN_DIR/$bin"
        log "  $BIN_DIR/$bin"
    done
}

do_install_binaries

# ─── 5. Config ────────────────────────────────────────────────────────
setup_config() {
    mkdir -p "$CONFIG_DIR"

    if [[ -f "$CONFIG_FILE" ]]; then
        log "Config exists at $CONFIG_FILE (skipping)"
    else
        log "Creating default config..."
        cat > "$CONFIG_FILE" <<'TOML'
# Aletheon configuration
# API keys: set in ~/.aletheon/.env or environment variables

[agent]
default_provider = "deepseek"
default_model = "deepseek-v4-flash"
max_iterations = 25
max_tokens = 100000

# ─── Providers ─────────────────────────────────────────────────────────
# api_key: set here or via env var <NAME>_API_KEY (e.g. DEEPSEEK_API_KEY)
# transport: "auto" (detect from URL) | "openai" | "anthropic"

[[providers]]
name = "deepseek"
base_url = "https://api.deepseek.com"
# api_key = ""
transport = "openai"
models = ["deepseek-v4-pro", "deepseek-v4-flash"]

[[providers]]
name = "anthropic"
base_url = "https://api.anthropic.com"
# api_key = ""
transport = "anthropic"
models = ["claude-sonnet-4-20250514"]

[[providers]]
name = "openai"
base_url = "https://api.openai.com/v1"
# api_key = ""
transport = "openai"
models = ["gpt-4o", "gpt-4o-mini"]

[[providers]]
name = "ollama"
base_url = "http://localhost:11434"
transport = "openai"
models = ["qwen3:8b"]

# ─── Model Aliases ─────────────────────────────────────────────────────
[model_aliases]
sonnet = "anthropic/claude-sonnet-4-20250514"
local = "ollama/qwen3:8b"
TOML
        log "Config written to $CONFIG_FILE"
    fi

    if [[ -f "$ENV_FILE" ]]; then
        log "Env file exists at $ENV_FILE (skipping)"
    else
        log "Creating placeholder .env..."
        cat > "$ENV_FILE" <<'ENV'
# Aletheon provider API keys
# Uncomment and fill in the keys you want to use

# DeepSeek
# DEEPSEEK_API_KEY=sk-...

# Anthropic
# ANTHROPIC_API_KEY=sk-ant-...

# OpenAI
# OPENAI_API_KEY=sk-...
ENV
        log "Env file written to $ENV_FILE"
    fi
}

setup_config

# ─── 6. Create wrapper ───────────────────────────────────────────────
setup_wrapper() {
    mkdir -p "$(dirname "$WRAPPER")"

    cat > "$WRAPPER" <<WRAPPER_EOF
#!/usr/bin/env bash
# Aletheon wrapper — generated by setup.sh
ALETHEON_CONFIG="\$HOME/.aletheon/config.toml"
ALETHEON_ENV="\$HOME/.aletheon/.env"
ALETHEON_SOCKET="$SOCKET_PATH"

export RUST_LOG="\${RUST_LOG:-info}"

usage() {
    cat <<EOF
Aletheon — self-evolving AI agent

Usage:
    aletheon -m <message>        Send a single message
    aletheon --tui               Interactive TUI mode
    aletheon status              Show daemon status
    aletheon start               Start daemon (sudo)
    aletheon stop                Stop daemon (sudo)
    aletheon restart             Restart daemon (sudo)
    aletheon logs                Follow daemon logs (sudo)
    aletheon --help              Show this help

Config:  \$ALETHEON_CONFIG
Env:     \$ALETHEON_ENV
Socket:  \$ALETHEON_SOCKET
EOF
}

case "\${1:-}" in
    start)
        sudo systemctl start $SERVICE_NAME
        sudo systemctl status $SERVICE_NAME --no-pager
        ;;
    stop)
        sudo systemctl stop $SERVICE_NAME
        ;;
    restart)
        sudo systemctl restart $SERVICE_NAME
        sudo systemctl status $SERVICE_NAME --no-pager
        ;;
    status)
        sudo systemctl status $SERVICE_NAME --no-pager 2>/dev/null || echo "Service not running"
        ;;
    logs)
        sudo journalctl -u $SERVICE_NAME -f
        ;;
    --help|-h|help)
        usage
        ;;
    *)
        exec $BIN_DIR/aletheon --socket "\$ALETHEON_SOCKET" "\$@"
        ;;
esac
WRAPPER_EOF

    chmod +x "$WRAPPER"
    log "Wrapper installed: $WRAPPER"
}

setup_wrapper

# ─── 7. Systemd service ──────────────────────────────────────────────
setup_systemd() {
    local svc="/etc/systemd/system/${SERVICE_NAME}.service"

    log "Creating system service: $svc"

    sudo tee "$svc" > /dev/null <<SERVICE_EOF
[Unit]
Description=Aletheon Agent Daemon
After=network.target

[Service]
Type=simple
User=$(whoami)
RuntimeDirectory=${SERVICE_NAME}
ExecStart=${BIN_DIR}/aletheond --socket ${SOCKET_PATH}
Restart=on-failure
RestartSec=5
EnvironmentFile=-${ENV_FILE}

[Install]
WantedBy=multi-user.target
SERVICE_EOF

    sudo systemctl daemon-reload
    sudo systemctl enable "$SERVICE_NAME" 2>/dev/null

    log "Service enabled: $SERVICE_NAME"
}

setup_systemd

# ─── 8. Start service ────────────────────────────────────────────────
do_start() {
    # Stop if already running
    if systemctl is-active --quiet "$SERVICE_NAME" 2>/dev/null; then
        log "Service already running, restarting with new binaries..."
        sudo systemctl restart "$SERVICE_NAME"
    else
        sudo systemctl start "$SERVICE_NAME"
    fi

    # Health check: wait for socket to appear
    log "Waiting for daemon to start..."
    local retries=10
    for i in $(seq 1 $retries); do
        if [[ -S "$SOCKET_PATH" ]]; then
            log "Daemon started, socket ready: $SOCKET_PATH"
            return 0
        fi
        sleep 1
    done

    warn "Socket not found after ${retries}s. Check: sudo journalctl -u $SERVICE_NAME -n 20"
}

do_start

# ─── Done ─────────────────────────────────────────────────────────────
echo ""
log "Setup complete!"
echo ""
echo "  Binaries:  $BIN_DIR/{aletheond,aletheon,aletheon-exec}"
echo "  Config:    $CONFIG_FILE"
echo "  Env:       $ENV_FILE"
echo "  Socket:    $SOCKET_PATH"
echo "  Service:   $SERVICE_NAME (systemd)"
echo "  Wrapper:   $WRAPPER"
echo ""
echo "  Quick start:"
echo "    aletheon -m 'hello'              # send a message"
echo "    aletheon --tui                    # interactive mode"
echo "    aletheon status                   # check service"
echo "    aletheon logs                     # follow logs"
echo ""
echo "  Edit config:"
echo "    \$EDITOR $CONFIG_FILE"
echo "    \$EDITOR $ENV_FILE"
echo ""
