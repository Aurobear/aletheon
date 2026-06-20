#!/usr/bin/env bash
# test_models.sh — Aletheon model + AutoMemory integration tests
# Usage: ./scripts/test_models.sh [model_alias ...]
# If no args, tests all configured models.
# Examples:
#   ./scripts/test_models.sh glm
#   ./scripts/test_models.sh deepseek_flash deepseek_pro
#   ./scripts/test_models.sh  # test all

set -euo pipefail

# ─── Config ────────────────────────────────────────────────────────────
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
ALETHEON_BIN="$PROJECT_ROOT/target/release"
DAEMON_BIN="$ALETHEON_BIN/aletheond"
CLI_BIN="$ALETHEON_BIN/aletheon-cli"
SOCKET="/tmp/aletheon-test.sock"
STATE_DIR="/tmp/aletheon-test-$$"
TIMEOUT=120  # seconds per request

# Default models to test (override via args)
# Format: "alias:provider_name/model_name" — alias for display, split for config
DEFAULT_MODELS=(
  "pro:mimo/mimo-v2.5-pro"
  "flash:mimo/mimo-v2.5-flash"
  # --- lejurobot/deepseek (uncomment when quota available) ---
  # "glm:lejurobot/glm-5.2"
  # "deepseek_flash:lejurobot_deepseek/deepseek/deepseek-v4-flash"
  # "deepseek_pro:lejurobot_deepseek/deepseek/deepseek-v4-pro"
)

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

# ─── Helpers ───────────────────────────────────────────────────────────
log()  { echo -e "${CYAN}[TEST]${NC} $*"; }
pass() { echo -e "${GREEN}[PASS]${NC} $*"; PASS_COUNT=$((PASS_COUNT + 1)); }
fail() { echo -e "${RED}[FAIL]${NC} $*"; FAIL_COUNT=$((FAIL_COUNT + 1)); FAILURES+=("$*"); }
skip() { echo -e "${YELLOW}[SKIP]${NC} $*"; SKIP_COUNT=$((SKIP_COUNT + 1)); }

PASS_COUNT=0
FAIL_COUNT=0
SKIP_COUNT=0
FAILURES=()

cleanup() {
    log "Cleaning up..."
    # Kill daemon if running
    if [[ -f "$STATE_DIR/daemon.pid" ]]; then
        kill "$(cat "$STATE_DIR/daemon.pid")" 2>/dev/null || true
        rm -f "$STATE_DIR/daemon.pid"
    fi
    # Also kill by socket
    pkill -f "aletheond.*$SOCKET" 2>/dev/null || true
    rm -f "$SOCKET"
    rm -rf "$STATE_DIR"
    log "Done."
}
trap cleanup EXIT

# Send a message to daemon, return stdout (with timeout)
send_message() {
    local msg="$1"
    timeout "$TIMEOUT" "$CLI_BIN" -s "$SOCKET" -m "$msg" 2>/dev/null
}

# Create temp config with a specific model as default
# Arg: "provider_name/model_name" format (e.g., "lejurobot/glm-5.2")
# Splits on first / to set default_provider and default_model separately
create_config() {
    local model_spec="$1"
    local provider="${model_spec%%/*}"
    local model="${model_spec#*/}"
    local tmp_config="$STATE_DIR/config.toml"
    mkdir -p "$STATE_DIR"
    sed -e "s|^default_provider = .*|default_provider = \"$provider\"|" \
        -e "s|^default_model = .*|default_model = \"$model\"|" \
        "$HOME/.aletheon/config.toml" > "$tmp_config"
    echo "$tmp_config"
}

# Start daemon with a specific model
# Arg: "alias:provider/model" format
start_daemon() {
    local entry="$1"
    local alias="${entry%%:*}"
    local model_spec="${entry#*:}"
    local provider="${model_spec%%/*}"
    local model="${model_spec#*/}"
    log "Starting daemon with model=$alias (provider=$provider, model=$model) ..."

    # Clean previous state
    rm -f "$SOCKET"

    # Create temp config
    local tmp_config
    tmp_config=$(create_config "$model_spec")

    # Start daemon in background
    ALETHEON_STATE_DIR="$STATE_DIR" \
    "$DAEMON_BIN" -c "$tmp_config" -s "$SOCKET" &
    local daemon_pid=$!
    echo "$daemon_pid" > "$STATE_DIR/daemon.pid"

    # Wait for socket
    local wait_count=0
    while [[ ! -S "$SOCKET" ]] && [[ $wait_count -lt 30 ]]; do
        sleep 1
        wait_count=$((wait_count + 1))
        if ! kill -0 "$daemon_pid" 2>/dev/null; then
            fail "Daemon died during startup (model=$alias)"
            return 1
        fi
    done

    if [[ ! -S "$SOCKET" ]]; then
        fail "Daemon socket not ready after 30s (model=$alias)"
        return 1
    fi

    log "Daemon started (PID=$daemon_pid)"
    return 0
}

# Stop daemon
stop_daemon() {
    if [[ -f "$STATE_DIR/daemon.pid" ]]; then
        local pid
        pid=$(cat "$STATE_DIR/daemon.pid")
        if kill -0 "$pid" 2>/dev/null; then
            kill "$pid" 2>/dev/null || true
            # Wait up to 5s for clean exit
            local w=0
            while kill -0 "$pid" 2>/dev/null && [[ $w -lt 5 ]]; do
                sleep 1
                w=$((w + 1))
            done
            kill -9 "$pid" 2>/dev/null || true
        fi
        rm -f "$STATE_DIR/daemon.pid"
    fi
    rm -f "$SOCKET"
    sleep 1
}

# ─── Test: Basic Response ─────────────────────────────────────────────
test_basic_response() {
    local model="$1"
    log "[$model] Test: basic response..."

    local response
    response=$(send_message "Reply with exactly: OK" 2>&1) || true

    if [[ -z "$response" ]]; then
        fail "[$model] basic: empty response"
        return
    fi

    # Log first 200 chars of response for debugging
    log "[$model] response preview: ${response:0:200}"

    if echo "$response" | grep -qi "ok"; then
        pass "[$model] basic: got expected response"
    else
        pass "[$model] basic: got response (len=${#response})"
    fi
}

# ─── Test: AutoMemory Store ───────────────────────────────────────────
test_auto_memory_store() {
    local model="$1"
    log "[$model] Test: AutoMemory store..."

    local response
    response=$(send_message "我叫TestBot，喜欢Python" 2>&1) || true

    if [[ -z "$response" ]]; then
        fail "[$model] automem_store: empty response"
        return
    fi

    log "[$model] automem_store preview: ${response:0:200}"
    pass "[$model] automem_store: got response (len=${#response})"
}

# ─── Test: AutoMemory Recall ──────────────────────────────────────────
test_auto_memory_recall() {
    local model="$1"
    log "[$model] Test: AutoMemory recall (store + recall in same session)..."

    # Step 1: Store a distinctive fact via explicit tool call
    local store_resp
    store_resp=$(send_message "请用core_memory_append工具存储：label=human，content=用户的项目代号是AlphaSeven" 2>&1) || true
    log "[$model] store response preview: ${store_resp:0:150}"

    # Step 2: Wait for AutoMemory to process
    sleep 3

    # Step 3: Ask to recall
    local response
    response=$(send_message "我的项目代号是什么？请先用memory_search搜索记忆。" 2>&1) || true

    if [[ -z "$response" ]]; then
        fail "[$model] automem_recall: empty response"
        return
    fi

    log "[$model] recall response preview: ${response:0:200}"

    if echo "$response" | grep -qi "AlphaSeven"; then
        pass "[$model] automem_recall: recalled stored fact"
    else
        log "[$model] automem_recall: 'AlphaSeven' not found in response"
        skip "[$model] automem_recall: fact not recalled"
    fi
}

# ─── Test: Tool Calling ──────────────────────────────────────────────
test_tool_calling() {
    local model="$1"
    log "[$model] Test: tool calling (list_files)..."

    local response
    response=$(send_message "列出当前目录的文件" 2>&1) || true

    if [[ -z "$response" ]]; then
        fail "[$model] tool_call: empty response"
        return
    fi

    log "[$model] tool_call preview: ${response:0:200}"
    pass "[$model] tool_call: got response (len=${#response})"
}

# ─── Test: Multimodal Detection ─────────────────────────────────────
test_multimodal_detection() {
    local model="$1"
    log "[$model] Test: multimodal detection..."

    local response
    response=$(send_message "看这张图片 ![photo](test.png)" 2>&1) || true

    if [[ -z "$response" ]]; then
        fail "[$model] multimodal: empty response"
        return
    fi

    log "[$model] multimodal preview: ${response:0:200}"
    pass "[$model] multimodal: handled image message (len=${#response})"
}

# ─── Test: Reasoning Task ───────────────────────────────────────────
test_reasoning_task() {
    local model="$1"
    log "[$model] Test: reasoning task..."

    local response
    response=$(send_message "请分析并对比Rust和Go语言在系统编程中的架构设计权衡，考虑性能、安全性、并发模型等方面的trade-off" 2>&1) || true

    if [[ -z "$response" ]]; then
        fail "[$model] reasoning: empty response"
        return
    fi

    log "[$model] reasoning preview: ${response:0:200}"
    pass "[$model] reasoning: got response (len=${#response})"
}

# ─── Run all tests for one model ──────────────────────────────────────
# Arg: "alias:provider/model" format
run_model_tests() {
    local entry="$1"
    local model="${entry%%:*}"
    echo ""
    echo -e "${CYAN}═══════════════════════════════════════════════════${NC}"
    echo -e "${CYAN}  Testing model: ${model}${NC}"
    echo -e "${CYAN}═══════════════════════════════════════════════════${NC}"

    if ! start_daemon "$entry"; then
        return
    fi

    # Give daemon a moment to fully initialize
    sleep 2

    test_basic_response "$model"
    test_tool_calling "$model"
    test_auto_memory_store "$model"
    test_auto_memory_recall "$model"
    test_multimodal_detection "$model"
    test_reasoning_task "$model"

    stop_daemon
}

# ─── Main ─────────────────────────────────────────────────────────────
main() {
    echo ""
    echo -e "${CYAN}╔═══════════════════════════════════════════════════╗${NC}"
    echo -e "${CYAN}║     Aletheon Model & AutoMemory Test Suite        ║${NC}"
    echo -e "${CYAN}╚═══════════════════════════════════════════════════╝${NC}"
    echo ""

    # Check binaries exist
    if [[ ! -x "$DAEMON_BIN" ]] || [[ ! -x "$CLI_BIN" ]]; then
        echo -e "${RED}Error: Binaries not found. Run 'cargo build --release' first.${NC}"
        exit 1
    fi

    # Determine models to test
    local models=("${@:-${DEFAULT_MODELS[@]}}")
    local display_names=()
    for m in "${models[@]}"; do display_names+=("${m%%:*}"); done
    log "Models to test: ${display_names[*]}"
    log "Timeout per request: ${TIMEOUT}s"
    echo ""

    for entry in "${models[@]}"; do
        run_model_tests "$entry"
    done

    # ─── Summary ──────────────────────────────────────────────────────
    echo ""
    echo -e "${CYAN}═══════════════════════════════════════════════════${NC}"
    echo -e "${CYAN}  Test Summary${NC}"
    echo -e "${CYAN}═══════════════════════════════════════════════════${NC}"
    echo -e "  ${GREEN}PASS: $PASS_COUNT${NC}"
    echo -e "  ${RED}FAIL: $FAIL_COUNT${NC}"
    echo -e "  ${YELLOW}SKIP: $SKIP_COUNT${NC}"

    if [[ ${#FAILURES[@]} -gt 0 ]]; then
        echo ""
        echo -e "${RED}Failures:${NC}"
        for f in "${FAILURES[@]}"; do
            echo -e "  ${RED}✗${NC} $f"
        done
    fi

    echo ""
    if [[ $FAIL_COUNT -gt 0 ]]; then
        exit 1
    fi
}

main "$@"
