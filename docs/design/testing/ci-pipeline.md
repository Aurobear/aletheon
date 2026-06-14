# CI 流水线 (CI Pipeline)

> 自动化测试、构建和发布流程。CI 设计就绪，GitHub Actions 配置待创建。

**关联模块:** [测试策略](test-strategy.md), [Mock 策略](mock-strategy.md)
**最后更新:** 2026-06-07 (B1-B5 merged)

---

## Implementation Status

| Component | Status | Code Location | Notes |
|-----------|--------|---------------|-------|
| CI Pipeline | 🟡 Design Ready | `.github/workflows/ci.yml` (待创建) | 设计完成，workflow 配置待落地 |
| Unit test suite | ✅ Implemented | `crates/agent-core/src/` | 533 tests pass |
| Mock infrastructure | ✅ Implemented | `crates/agent-core/src/testing/` | MockLlm, MockSandbox, MockMemory, MockPerception |

---

## 1. 流水线阶段

### 1.1 已配置的 CI 阶段 (待落地到 GitHub Actions)

计划的 workflow 包含以下 job:

```yaml
# .github/workflows/ci.yml (待创建)
jobs:
  - python-lint:    # Python 代码 lint (scripts/, src/lib/)
  - shell-lint:     # Shell 脚本 lint (scripts/)
  - pytest:         # Python 测试 (src/lib/ 下的 pytest 套件)
  - unit-tests:     # cargo test --all — 533+ Rust 单元测试
  - validate:       # bash scripts/aurb.sh validate — 打包验证
  - integration:    # 集成测试
  - ci-status:      # 汇总所有 job 状态
```

### 1.2 扩展阶段 (未来)

```yaml
stages:
  - special-test:
      - cargo test --features ebpf-tests   # eBPF 测试 (需要 root)
      - cargo test --features sandbox-tests # 沙箱测试 (需要 bubblewrap)
      - cargo test --features fuse-tests    # FUSE 测试 (需要 fuse3)

  - build:
      - cargo build --release
      - cargo build --release --features embed-llama  # 带本地推理构建
      - 构建 eBPF 程序 (clang + llvm)

  - package:
      - 构建 Arch Linux 包 (.pkg.tar.zst)
      - 构建 .deb
      - DKMS 打包 (内核模块)

  - security-scan:
      - cargo audit                    # 依赖漏洞扫描
      - cargo deny check               # 许可证合规性
      - cargo udeps                    # 未使用依赖检测

  - benchmark:
      - cargo bench                    # criterion 基准测试
      - 比较与前一次运行的性能差异

  - deploy:
      - 推送到测试环境
      - 运行端到端测试
```

---

## 2. GitHub Actions 配置设计

### 2.1 Workflow 矩阵

```yaml
jobs:
  test:
    strategy:
      matrix:
        rust: [stable, nightly]
        os: [ubuntu-latest, ubuntu-24.04-arm]
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy, rustfmt
      - run: cargo clippy -- -D warnings
      - run: cargo test --all
```

### 2.2 Runner 分类

| Runner | 用途 | 需求 |
|--------|------|------|
| `ubuntu-latest` | lint + 单元测试 + 集成测试 | 普通 |
| `self-hosted` | eBPF + 沙箱 + FUSE 测试 | root 权限 |
| `self-hosted-bench` | 基准测试 | 专用裸机 |
| `ubuntu-24.04-arm` | ARM 交叉编译验证 | ARM runner |

### 2.3 条件触发

| 触发条件 | 运行内容 | 理由 |
|----------|----------|------|
| push to dev/* | 完整流水线 | 开发中分支 |
| push to main | 完整流水线 + deploy | 主分支 |
| PR to main | lint + test + security-scan | 不部署 |
| nightly cron | benchmark + integration | 长期趋势 |
| tag v* | 完整流水线 + package + release | 发布版本 |

---

## 3. 特殊环境配置

### 3.1 eBPF 测试环境

```yaml
- run: |
    # 安装内核头文件和 libbpf
    apt-get update && apt-get install -y linux-headers-$(uname -r) libbpf-dev
    # 编译 eBPF 程序
    cd crates/agent-ebpf && cargo build --features ebpf-tests
    # 需要 root 权限
    sudo -E cargo test --features ebpf-tests
```

### 3.2 沙箱测试环境

```yaml
- run: |
    # 确保 bubblewrap 可用
    which bwrap || apt-get install -y bubblewrap
    # user namespace 检查
    unshare --user true || echo "user namespace not available"
    # 运行沙箱测试
    cargo test --features sandbox-tests
```

### 3.3 FUSE 测试环境

```yaml
- run: |
    # 安装 fuse3
    apt-get install -y fuse3 libfuse3-dev
    # 加载 fuse 内核模块
    sudo modprobe fuse
    # 运行 FUSE 测试
    cargo test --features fuse-tests
```

---

## 4. 构建产物

| 产物 | 格式 | 包含 | 分发方式 |
|------|------|------|----------|
| agentd + agent-cli | `.pkg.tar.zst` | 二进制 + systemd service + 默认配置 | AUR |
| agentd + agent-cli | `.deb` | 二进制 + systemd service + 默认配置 | 手动 |
| DKMS 包 | `.deb` / `.pkg.tar.zst` | `agent_ipc.ko` 内核模块 | 手动 |
| Docker 镜像 | `ghcr.io/aurobear/aletheon` | aletheond + base tools | GHCR |

---

## 5. 发布流程

```
tag v0.x.x
    │
    ▼
  CI: lint + test + build
    │
    ▼
  CI: package (Arch/DEB/Docker)
    │
    ▼
  CI: 生成 release notes (changelog)
    │
    ▼
  CI: 上传产物到 GitHub Release
    │
    ▼
  通知: AUR 包自动更新
```

---

## 6. 参考来源

| 来源 | 借鉴内容 |
|------|----------|
| Rust CI 最佳实践 | clippy + deny + udeps 安全扫描流水线 |
| Codex | 自托管 runner 矩阵 + 权限测试环境 |
| OpenCode | 条件触发 CI（branch/PR/tag 不同策略） |
| Hermes Agent | 基准测试 CI runner 专用化 |
| bubblewrap 项目 | 沙箱 CI 环境配置检查脚本 |
| GitHub Actions 社区 | release-please + release CI 模板 |

---

## Implementation Summary

> CI 流水线设计就绪，GitHub Actions workflow 待创建。测试基础设施已就位。

| Component | Status | Notes |
|-----------|--------|-------|
| CI Pipeline | 🟡 Design Ready | `.github/workflows/ci.yml` 待创建，设计阶段已完成 |
| Unit test suite | ✅ Implemented | 533 tests pass in agent-core |
| Mock infrastructure | ✅ Implemented | MockLlm, MockSandbox, MockMemory, MockPerception |
| Build/package | 未实现 | 无 .deb / .pkg 构建脚本 |
| E2E tests | 未实现 | — |
| Security scanning | 未实现 | cargo-audit / deny / udeps 未集成 |
| Benchmarks | 未实现 | criterion 未集成到 CI |
