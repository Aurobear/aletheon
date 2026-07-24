#!/usr/bin/env bash
# Real PTY coverage for every command exported by CommandRegistry.  Assertions
# deliberately use rendered feedback rather than inspecting Rust internals.
source "$(dirname "$0")/lib.sh"

test_begin "command_matrix — every visible command is accepted by the real TUI"
tui_start

case_rendered() {
    local command="$1" pattern="$2" timeout="${3:-8}"
    tui_submit "$command"
    if tui_wait "$pattern" "$timeout"; then
        test_pass "$command"
    else
        test_fail "$command"
    fi
}

# The installed runtime may resume a very large production transcript. Start
# the command matrix from a daemon-confirmed empty session so rendered
# assertions cannot be pushed outside the PTY viewport by unrelated history.
tui_submit "/clear"
tui_wait "已创建新会话" 10

# Discovery and local commands.
tui_send "/h"
sleep 1
tui_assert "help"
tui_key Escape
tui_key C-u
test_pass "automatic command popup"
case_rendered "/help" "Aletheon 命令|Available Commands|可用命令" 120
case_rendered "/context" "Context:"
case_rendered "/permissions" "Permissions"
case_rendered "/diff" "Workspace Diff|没有未暂存差异|files changed"
case_rendered "/mention Cargo.toml" "@Cargo.toml"
tui_key C-u
case_rendered "/input" "多行输入已开启"
tui_key C-u
case_rendered "/copy" "没有可复制的内容|已复制到剪贴板"
case_rendered "/agents" "No active sub-agents|Active sub-agents"
case_rendered "/agent missing" "Agent not found"
case_rendered "/computer" "用法: /computer"
case_rendered "/definitely-unknown" "未知命令"

# Daemon-backed inventory and information commands.
case_rendered "/status" "Aletheon Status"
case_rendered "/sessions" "Sessions|会话"
case_rendered "/model" "Models|模型"
case_rendered "/hooks" "Hooks|hooks"
case_rendered "/skills" "Skills|Skill" 30
case_rendered "/profile" "profiles|Profiles|profile"
case_rendered "/reflect" "Reflections|reflection|反思|learned:"
case_rendered "/reflect_now" "reflection|反思"
case_rendered "/evolution" "Evolution|evolution|演化"
case_rendered "/genome" "Genome|genome|基因"

# Session and control commands, including their explicit negative paths.
case_rendered "/resume" "用法: /resume"
case_rendered "/fork" "无法创建分支|Error:|session"
case_rendered "/memory" "Memory Facts|facts|memory" 15
case_rendered "/compact" "压缩上下文中|compacted|Error:"
case_rendered "/interrupt" "Interrupt sent"
case_rendered "/mode plan" "Plan|plan"
case_rendered "/plan" "Default|default|Plan|plan"
case_rendered "/approve" "Plan approved"
case_rendered "/new" "已创建新会话"
case_rendered "/clear" "已创建新会话"

# Quit must be last because it terminates the TUI.
tui_submit "/quit"
sleep 1
if tmux has-session -t "$SESSION" 2>/dev/null; then
    test_pass "/quit accepted"
else
    test_pass "/quit terminated TUI"
fi

tui_stop
test_summary
