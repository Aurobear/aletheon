# ── Aletheon Dev Tasks ──────────────────────────────────────────────────
# cargo 自带增量编译，只编译变更的 crate 及其下游依赖。
# 日常开发用 dev（debug，秒级），部署前用 build（release + 全验证）。

default:
    @just --list

# ── 构建 ───────────────────────────────────────────────────────────────

# 快速增量编译（debug 模式，日常开发用）
dev:
    cargo build -p aletheon-bin

# 编译 + 测试 + lint 全部通过后才 build release
build: test lint
    cargo build -p aletheon-bin --release

# 查看各 crate 编译耗时
timings:
    cargo build --timings

# ── 验证 ───────────────────────────────────────────────────────────────

# 运行所有测试
test:
    cargo test --workspace

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

# CI 级全量验证 (fmt + test + lint + doc)
check: fmt test lint doc
    @echo "=== ALL CHECKS PASSED ==="

# 架构依赖、遗留路径和绕过调用只能减少，不能新增
architecture-check:
    cargo test -p executive --test layered_config_contract checked_in_schema_is_deterministic
    bash tests/architecture_check.sh
    bash tests/architecture_path_inventory.sh
    bash scripts/architecture-check.sh

# Deterministic cross-domain causal, isolation, replay and ablation evidence.
acceptance: architecture-check
    python3 tools/acceptance_report.py --check
    CARGO_INCREMENTAL=0 cargo test -j1 -p executive --test cross_domain_acceptance
    CARGO_INCREMENTAL=0 cargo test -j1 -p executive --test functional_indicators
    python3 tools/acceptance_report.py

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
    @mkdir -p .cargo
    @if ! grep -q 'rustc-wrapper' .cargo/config.toml 2>/dev/null; then \
        echo '[build]' >> .cargo/config.toml; \
        echo 'rustc-wrapper = "sccache"' >> .cargo/config.toml; \
    fi
    @echo "sccache configured in .cargo/config.toml"
