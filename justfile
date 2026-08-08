# ── Aletheon Dev Tasks ──────────────────────────────────────────────────
# cargo 自带增量编译，只编译变更的 crate 及其下游依赖。
# 日常开发用 dev（debug，秒级），部署前用 build（release + 全验证）。

default:
    @just --list

# ── 构建 ───────────────────────────────────────────────────────────────

# 快速增量类型检查（debug 模式，日常开发用，不支付链接成本）
# 例如 `just dev executive` 只检查该 crate 及其依赖。
dev package="aletheon":
    bash scripts/cargo-agent.sh check -p {{package}}

# 需要可执行文件时再做增量 debug 编译与链接
dev-build package="aletheon":
    bash scripts/cargo-agent.sh build -p {{package}}

# 编译 + 测试 + lint 全部通过后才 build release
build: test lint
    bash scripts/cargo-agent.sh build -p aletheon --release

# 查看各 crate 编译耗时
timings package="aletheon":
    bash scripts/cargo-agent.sh build -p {{package}} --timings

# ── 验证 ───────────────────────────────────────────────────────────────

# 运行所有测试
test:
    bash scripts/cargo-agent.sh test --workspace

# 只编译并运行一个 crate 的 lib 单元测试；不会枚举其集成测试二进制。
test-lib package:
    bash scripts/cargo-agent.sh test -p {{package}} --lib

# 只编译并运行一个明确的集成测试目标。
test-one package target:
    bash scripts/cargo-agent.sh test -p {{package}} --test {{target}}

# 只对一个 crate 做 all-targets 严格 lint。
lint-one package:
    bash scripts/cargo-agent.sh clippy -p {{package}} --all-targets -- -D warnings

# clippy 严格模式
lint:
    bash scripts/cargo-agent.sh clippy --workspace --all-targets -- -D warnings

# 格式化检查
fmt:
    bash scripts/cargo-agent.sh fmt --all -- --check

# 自动修复格式 + clippy 建议
fix:
    bash scripts/cargo-agent.sh fmt --all
    bash scripts/cargo-agent.sh clippy --workspace --all-targets --fix --allow-dirty --allow-no-vcs

# 生成文档
doc:
    RUSTDOCFLAGS="-D warnings" bash scripts/cargo-agent.sh doc --workspace --no-deps

# CI 级全量验证 (fmt + test + lint + doc)
check: fmt test lint doc
    @echo "=== ALL CHECKS PASSED ==="

# 架构依赖、遗留路径和绕过调用只能减少，不能新增
architecture-check:
    bash scripts/cargo-agent.sh test -p executive --test layered_config_contract checked_in_schema_is_deterministic
    bash scripts/aletheon.sh test architecture

# Deterministic cross-domain causal, isolation, replay and ablation evidence.
acceptance: architecture-check
    python3 tools/acceptance_report.py --check
    bash scripts/cargo-agent.sh test -j1 -p executive --test cross_domain_acceptance
    bash scripts/cargo-agent.sh test -j1 -p executive --test functional_indicators
    python3 tools/acceptance_report.py

# ── 部署 ───────────────────────────────────────────────────────────────

# 编译 release + 部署到系统
install: build
    sudo bash setup.sh

# ── 清理 ───────────────────────────────────────────────────────────────

# 删除编译缓存
clean:
    bash scripts/aletheon.sh cleanup cargo

# ── 加速（可选） ────────────────────────────────────────────────────────

# 安装 sccache 跨构建共享缓存（clean 后重编译快 50%+）
setup-sccache:
    cargo install sccache --locked
    @echo "sccache installed; scripts/cargo-agent.sh will detect it automatically"

# V02: installed-host production migration, scenario, failure and rollback gate.
# This invokes V01 through scripts/aletheon.sh acceptance release and fails closed when
# the disposable host, release binary, live credentials or operator are absent.
release-acceptance:
    bash scripts/aletheon.sh acceptance release
