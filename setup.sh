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

# ── 3. Build ─────────────────────────────────────────────────────────────

# Clean stale build artifacts from old binaries (removed [[bin]] entries)
for stale_bin in aletheond aletheon-exec aletheon-systemd aletheon-container; do
    rm -f "target/release/$stale_bin" "target/debug/$stale_bin"
done
log "Cleaned stale build artifacts"

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
    $USE_SUDO mkdir -p "$CFG_DIR" "$DATA_DIR"
    if [[ "$MODE" == "system" ]]; then
        $USE_SUDO chown -R "$(whoami):$(whoami)" "$DATA_DIR" 2>/dev/null || true
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
if [[ "$MODE" == "system" ]]; then
    echo "  Quick start:"
    echo "    sudo systemctl start aletheon"
    echo "    aletheon                     # launch TUI"
    echo "    aletheon daemon              # foreground debug"
    echo "    aletheon exec -p 'hello'     # non-interactive run"
else
    echo "  Quick start:"
    echo "    systemctl --user start aletheon"
    echo "    aletheon                     # launch TUI"
    echo "    aletheon daemon              # foreground debug"
    echo "    aletheon exec -p 'hello'     # non-interactive run"
fi
echo ""
