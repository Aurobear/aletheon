# Wave 5：Profiles 与评测门禁

**状态：** 被阻塞。**解锁条件：** W4-13 通过。

| ID | 实现 | 固定结果 | 验证 |
|---|---|---|---|
| W5-01 | executive config 新增 DeploymentProfile | 六种 kind 固定 core/coding/personal/conscious/evolution/hardware-edge | `bash scripts/cargo-agent.sh test -p executive deployment_profile` |
| W5-02 | RequiredTurnPorts 与 OptionalTurnFeatures | required 缺失拒绝 ready；非 required 状态写 manifest | `bash scripts/cargo-agent.sh test -p executive feature_ports` |
| W5-03 | bootstrap 构造门禁 | 先解析 profile，再打开领域库与服务 | `bash scripts/cargo-agent.sh test -p executive profile_gating` |
| W5-04 | capability/storage/recovery manifests | 从运行时真实注册表生成，禁止静态伪造 | `bash scripts/cargo-agent.sh test -p executive deployment_manifest` |
| W5-05 | ablation harness | memory/agora/dasein/metacog/workspace/recurrence 全覆盖，固定 seed | `bash scripts/cargo-agent.sh test -p executive ablations` |
| W5-06 | 30 个 coding fixtures | 13 类全覆盖、id 唯一、每 fixture 少于 50 文件 | `bash tests/benchmarks/coding/manifest_static_test.sh` |
| W5-07 | coding-bench crate 与指标 | receipt schema、算法和校验固定 | `bash scripts/cargo-agent.sh test -p coding-bench` |
| W5-08 | release gate | 阈值固定 0.60/0.80/0.05/0/0.90 | `bash tests/production/release_aggregate_receipt_test.sh` |
| W5-09 | 六个 config 文件与架构登记 | 全部能被 AppConfig 解析 | `bash scripts/cargo-agent.sh test -p executive profile_configs` |
| W5-10 | profile × benchmark 矩阵 | core/coding 冒烟、全量 release gate | `bash scripts/cargo-agent.sh test -p coding-bench --test profile_matrix` |

Ablation 不决定是否继续本 Wave；它只决定某层能否进入默认 coding profile。没有可重复增益的层固定保持关闭。`hardware-edge` 在 Hardware D0 未完成前必须拒绝 ready，禁止输出成功 manifest。
