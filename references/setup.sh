#!/usr/bin/env bash
# OS Agent References — 一键下载论文 + 克隆项目
# 用法: bash setup.sh [--papers] [--projects] [--nous] [--all] [--shallow]
# 默认: --all --shallow

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PAPERS_DIR="$SCRIPT_DIR/papers"
SHALLOW=true
DO_PAPERS=false
DO_PROJECTS=false
DO_NOUS=false

# ── 参数解析 ──
for arg in "$@"; do
  case "$arg" in
    --papers)   DO_PAPERS=true ;;
    --projects) DO_PROJECTS=true ;;
    --nous)     DO_NOUS=true ;;
    --all)      DO_PAPERS=true; DO_PROJECTS=true; DO_NOUS=true ;;
    --full)     SHALLOW=false ;;
    --shallow)  SHALLOW=true ;;
    -h|--help)
      echo "用法: bash setup.sh [--papers] [--projects] [--nous] [--all] [--shallow] [--full]"
      echo "  --papers    只下载论文"
      echo "  --projects  只克隆项目"
      echo "  --nous      下载 Nous 架构参考 (哲学 + 认知科学 + Agent 理论)"
      echo "  --all       全部下载 (默认)"
      echo "  --shallow   浅克隆 (默认)"
      echo "  --full      完整克隆"
      exit 0 ;;
    *) echo "未知参数: $arg"; exit 1 ;;
  esac
done

# 默认 --all
$DO_PAPERS || $DO_PROJECTS || $DO_NOUS || { DO_PAPERS=true; DO_PROJECTS=true; DO_NOUS=true; }

# ── 并发控制 ──
MAX_PARALLEL=4
running=0
pids=()

wait_jobs() {
  for pid in "${pids[@]}"; do
    wait "$pid" 2>/dev/null || true
  done
  pids=()
  running=0
}

run_bg() {
  if (( running >= MAX_PARALLEL )); then
    wait_jobs
  fi
  "$@" &
  pids+=($!)
  ((running++))
}

# ── 论文下载 ──
download_paper() {
  local url="$1" filename="$2"
  if [[ -f "$PAPERS_DIR/$filename" ]]; then
    echo "  [跳过] $filename (已存在)"
    return 0
  fi
  echo "  [下载] $filename"
  curl -L --fail -o "$PAPERS_DIR/$filename" "$url" 2>/dev/null && \
    echo "  [完成] $filename" || \
    echo "  [失败] $filename — 请手动下载: $url"
}

# ── 项目克隆 ──
clone_project() {
  local url="$1" dest="$2" category="$3"
  local target_dir="$SCRIPT_DIR/$category/$dest"
  if [[ -d "$target_dir" ]]; then
    echo "  [跳过] $dest (已存在)"
    return 0
  fi
  echo "  [克隆] $dest → $category/"
  local clone_args=(--depth 1)
  $SHALLOW || clone_args=()
  git clone "${clone_args[@]}" "$url" "$target_dir" 2>/dev/null && \
    echo "  [完成] $dest" || \
    echo "  [失败] $dest — 请手动克隆: $url"
}

echo ""
echo "═══════════════════════════════════════"
echo "  OS Agent + Nous References Setup"
echo "═══════════════════════════════════════"
echo ""

# ── 下载论文 ──
if $DO_PAPERS; then
  echo "── 论文下载 ──"
  mkdir -p "$PAPERS_DIR"

  download_paper "https://arxiv.org/pdf/2410.08164" "Agent_S_2410.08164.pdf" &
  download_paper "https://arxiv.org/pdf/2504.00906" "Agent_S2_2504.00906.pdf" &
  download_paper "https://arxiv.org/pdf/2407.16741" "OpenHands_2407.16741.pdf" &
  download_paper "https://arxiv.org/pdf/2404.07972" "OSWorld_2404.07972.pdf" &
  wait
  echo ""
fi

# ── Nous 架构参考 ──
if $DO_NOUS; then
  echo "── Nous 架构参考 ──"

  # Philosophy (Soul layer foundations)
  echo "  [哲学 — Soul 层理论基础]"
  mkdir -p "$SCRIPT_DIR/philosophy"

  # Spinoza — Ethics (conatus, public domain)
  download_paper "https://www.gutenberg.org/files/3800/3800-0.txt" \
    "$SCRIPT_DIR/philosophy/spinoza-ethics.txt" 2>/dev/null || true

  # Spinoza — Stanford Encyclopedia (modal metaphysics, conatus)
  run_bg download_paper "https://plato.stanford.edu/entries/spinoza-modal/" \
    "$SCRIPT_DIR/philosophy/spinoza-modal-sep.html"

  # Heidegger — Stanford Encyclopedia (Being and Time, Dasein)
  run_bg download_paper "https://plato.stanford.edu/entries/heidegger/" \
    "$SCRIPT_DIR/philosophy/heidegger-being-and-time-sep.html"

  # Dennett — Self as Center of Narrative Gravity
  run_bg download_paper "https://ase.tufts.edu/cogstud/dennett/papers/selfgravity.pdf" \
    "$SCRIPT_DIR/philosophy/dennett-narrative-gravity.pdf"

  # Dennett — Multiple Drafts model
  run_bg download_paper "https://ase.tufts.edu/cogstud/dennett/papers/multipleDrafts.pdf" \
    "$SCRIPT_DIR/philosophy/dennett-multiple-drafts.pdf"

  # Metzinger — PhilArchive entry (Being No One)
  run_bg download_paper "https://philarchive.org/rec/METBNO" \
    "$SCRIPT_DIR/philosophy/metzinger-being-no-one-philarchive.html"

  # Metzinger — The Ego Tunnel (preview)
  run_bg download_paper "https://www.thomasmetzinger.de/media/articles/Metzinger_2009_Ego-Tunnel.pdf" \
    "$SCRIPT_DIR/philosophy/metzinger-ego-tunnel-preview.pdf"

  wait_jobs
  echo ""

  # Cognitive Architecture (Brain layer design references)
  echo "  [认知科学 — Brain 层设计参考]"
  mkdir -p "$SCRIPT_DIR/cognitive-architecture"

  # OpenCog — README
  run_bg download_paper "https://raw.githubusercontent.com/opencog/opencog/master/README.md" \
    "$SCRIPT_DIR/cognitive-architecture/opencog-readme.md"

  # SOAR — Laird 2012 comprehensive paper
  run_bg download_paper "https://soar.eecs.umich.edu/wp-content/uploads/2022/08/2012_The-Soar-Cognitive-Architecture_Laird.pdf" \
    "$SCRIPT_DIR/cognitive-architecture/soar-laird-2012.pdf"

  # ACT-R — Anderson et al. 2004 Integrated Theory
  run_bg download_paper "https://act-r.psy.cmu.edu/wordpress/wp-content/uploads/2012/04/705Anderson2004.pdf" \
    "$SCRIPT_DIR/cognitive-architecture/anderson-2004-integrated-theory.pdf"

  # ACT-R — Anderson 2007 chapter
  run_bg download_paper "https://act-r.psy.cmu.edu/wordpress/wp-content/uploads/2012/04/709chapter.pdf" \
    "$SCRIPT_DIR/cognitive-architecture/anderson-2007-chapter.pdf"

  wait_jobs
  echo ""

  # Agent Theory (Brain reasoning + reflection references)
  echo "  [Agent 理论 — 推理与反思]"
  mkdir -p "$SCRIPT_DIR/agent"

  # ReAct — Yao et al. 2022
  run_bg download_paper "https://arxiv.org/pdf/2210.03629" \
    "$SCRIPT_DIR/agent/react-yao-2022.pdf"

  # Reflexion — Shinn et al. 2023
  run_bg download_paper "https://arxiv.org/pdf/2303.11366" \
    "$SCRIPT_DIR/agent/reflexion-shinn-2023.pdf"

  # Agent S — Simular 2024 (GUI agent)
  run_bg download_paper "https://arxiv.org/pdf/2410.08164" \
    "$SCRIPT_DIR/agent/agent-s-2024.pdf"

  # Agent S2 — Simular 2025
  run_bg download_paper "https://arxiv.org/pdf/2504.00915" \
    "$SCRIPT_DIR/agent/agent-s2-2025.pdf"

  # K²-Agent
  run_bg download_paper "https://arxiv.org/pdf/2410.18184" \
    "$SCRIPT_DIR/agent/k2-agent-2024.pdf"

  wait_jobs
  echo ""
fi

# ── 克隆项目 ──
if $DO_PROJECTS; then
  echo "── 项目克隆 ──"
  $SHALLOW && echo "  (浅克隆模式, 使用 --full 获取完整历史)"
  echo ""

  # Computer Agent / OS Agent
  echo "  [Computer Agent / OS Agent]"
  mkdir -p "$SCRIPT_DIR/projects"
  run_bg clone_project "https://github.com/simular-ai/Agent-S.git" "Agent-S" "projects"
  run_bg clone_project "https://github.com/OpenHands/open-operator.git" "open-operator" "projects"
  run_bg clone_project "https://github.com/openclaw/openclaw.git" "openclaw" "projects"
  wait_jobs

  # Agent Runtime
  echo "  [Agent Runtime]"
  mkdir -p "$SCRIPT_DIR/runtime"
  run_bg clone_project "https://github.com/All-Hands-AI/OpenHands.git" "OpenHands" "runtime"
  wait_jobs

  # Agent Framework
  echo "  [Agent Framework]"
  mkdir -p "$SCRIPT_DIR/agent-framework"
  run_bg clone_project "https://github.com/langchain-ai/langgraph.git" "langgraph" "agent-framework"
  run_bg clone_project "https://github.com/microsoft/autogen.git" "autogen" "agent-framework"
  run_bg clone_project "https://github.com/crewAIInc/crewAI.git" "crewAI" "agent-framework"
  run_bg clone_project "https://github.com/letta-ai/letta.git" "letta" "agent-framework"
  run_bg clone_project "https://github.com/NousResearch/hermes-agent.git" "hermes-agent" "agent-framework"
  wait_jobs

  # CLI / Coding Agent
  echo "  [CLI / Coding Agent]"
  mkdir -p "$SCRIPT_DIR/cli-agent"
  run_bg clone_project "https://github.com/nicepkg/opencode.git" "opencode" "cli-agent"
  run_bg clone_project "https://github.com/anthropics/claude-code.git" "claude-code" "cli-agent"
  run_bg clone_project "https://github.com/openai/codex.git" "codex" "cli-agent"
  run_bg clone_project "https://github.com/esengine/DeepSeek-Reasonix.git" "DeepSeek-Reasonix" "cli-agent"
  wait_jobs

  # SDK
  echo "  [SDK]"
  mkdir -p "$SCRIPT_DIR/sdk"
  run_bg clone_project "https://github.com/anthropics/anthropic-sdk-python.git" "anthropic-sdk-python" "sdk"
  wait_jobs

  echo ""
fi

echo "═══════════════════════════════════════"
echo "  完成!"
echo "═══════════════════════════════════════"
echo ""

# ── 统计 ──
if $DO_PAPERS; then
  paper_count=$(find "$PAPERS_DIR" -name "*.pdf" 2>/dev/null | wc -l)
  paper_size=$(du -sh "$PAPERS_DIR" 2>/dev/null | cut -f1)
  echo "  论文: ${paper_count} 篇, ${paper_size}"
fi

if $DO_PROJECTS; then
  proj_count=$(find "$SCRIPT_DIR" -mindepth 2 -maxdepth 3 -name ".git" 2>/dev/null | wc -l)
  echo "  项目: ${proj_count} 个"
fi

if $DO_NOUS; then
  phil_count=$(find "$SCRIPT_DIR/philosophy" -type f 2>/dev/null | wc -l)
  cog_count=$(find "$SCRIPT_DIR/cognitive-architecture" -type f 2>/dev/null | wc -l)
  agent_theory_count=$(find "$SCRIPT_DIR/agent" -type f 2>/dev/null | wc -l)
  echo "  Nous 参考: 哲学 ${phil_count} + 认知科学 ${cog_count} + Agent 理论 ${agent_theory_count}"
fi

echo ""
echo "  目录结构:"
echo "  papers/                  — 论文 PDF (Agent S, OpenHands, OSWorld)"
echo "  philosophy/              — 哲学 (Spinoza, Heidegger, Dennett, Metzinger)"
echo "  cognitive-architecture/  — 认知科学 (OpenCog, SOAR, ACT-R)"
echo "  agent/                   — Agent 理论 (ReAct, Reflexion, Agent S/S2, K²-Agent)"
echo "  projects/                — Computer Agent / OS Agent"
echo "  runtime/                 — Agent Runtime"
echo "  agent-framework/         — Agent 框架"
echo "  cli-agent/               — CLI 编程 Agent"
echo "  sdk/                     — SDK"
echo ""
