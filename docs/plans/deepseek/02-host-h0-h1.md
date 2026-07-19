# Host Platform H0–H1

**状态：** 被阻塞。**解锁条件：** W1-06 验收通过。
**上游：** `docs/arch/aletheon-host-platform-plan.md:28-174`、`docs/plans/2026-07-19-host-platform.md:42-191`。

## 固定架构

依赖方向固定为 `platform-api <- platform-host <- platform-linux`。`platform-api` 放小型 traits、receipt、错误、路径和 `structured_patch`；`platform-host` 只做 backend 选择；Linux 系统调用只存在于 `platform-linux`。

## 唯一任务序列

| ID | 修改范围 | 完成条件 | 验证 |
|---|---|---|---|
| H0-01 | 创建 `crates/platform-api/{Cargo.toml,src/lib.rs,error.rs,path.rs,receipt.rs}`；加入 workspace | 类型 serde 往返；无 OS 依赖 | `bash scripts/cargo-agent.sh test -p platform-api` |
| H0-02 | 创建 `process.rs,filesystem.rs,pty.rs,service.rs,sandbox.rs,desktop.rs` traits | 每个 trait 单一职责；禁止巨型 adapter | 同上 |
| H0-03 | 创建 `manifest.rs`，定义 capability、support 状态和 probe receipt | 缺失能力返回 `Unsupported`，禁止静默 fallback | 同上 |
| H0-04 | 把 `structured_patch` 从 corpus 移到 `platform-api/src/structured_patch.rs` | corpus 与 exec-server 复用同一实现 | `bash scripts/cargo-agent.sh test -p platform-api structured_patch` |
| H0-05 | 创建 `platform-host` registry/selector | backend 必须由 manifest 与目标 OS 唯一决定 | `bash scripts/cargo-agent.sh test -p platform-host` |
| H0-06 | 旧 `PlatformAdapter` 改为零逻辑 facade并登记删除期限 | 新代码禁止引用旧 facade | `bash tests/architecture_check.sh` |
| H0-07 | CI 加 Linux build 及 Windows/macOS compile-only | 三 target 的 platform-api 编译 | CI receipt 三项 PASS |
| H1-01 | 创建 `platform-linux` 与 probe | cgroup v2、pidfd、inotify、PTY、systemd 状态可诊断 | `bash scripts/cargo-agent.sh test -p platform-linux probe` |
| H1-02 | 实现 ProcessHost：pidfd、进程组、timeout、cancel | cancel 后进程树为 0 | `bash scripts/cargo-agent.sh test -p platform-linux process_host` |
| H1-03 | 实现 FilesystemHost：root confinement、inotify | escape 拒绝，事件稳定排序 | `bash scripts/cargo-agent.sh test -p platform-linux filesystem_host` |
| H1-04 | 实现 Unix PTY | resize、EOF、cancel receipt 正确 | `bash scripts/cargo-agent.sh test -p platform-linux pty_host` |
| H1-05 | 实现 systemd D-Bus ServiceHost，固定使用 `zbus` | 无 systemd 时返回 Unsupported | `bash scripts/cargo-agent.sh test -p platform-linux service_host` |
| H1-06 | 迁移现有 sandbox_driver 原语 | namespace/seccomp/cgroup fail-closed | `bash scripts/cargo-agent.sh test -p platform-linux sandbox_host` |
| H1-07 | Workspace tools、Pi、Dasein command 改经 Host traits | 上层不直接调用 OS API | `bash tests/architecture_check.sh` |
| H1-08 | contract suite 对 platform-host 与 linux backend 运行 | 所有 traits 共享测试全绿 | `bash scripts/cargo-agent.sh test -p platform-host --test contract` |

每个 ID 单独提交。H0-01 未完成前禁止创建 H0-02；依此顺序执行。
