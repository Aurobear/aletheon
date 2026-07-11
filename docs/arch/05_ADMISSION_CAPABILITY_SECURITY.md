# Phase 5：Admission、Capability 与 Security

## 1. 目标

所有会产生外部副作用、消耗预算或触达高风险资源的能力调用，都必须先经过统一 Admission。安全策略不能只靠 prompt note、日志警告或调用方自觉遵守。

本阶段完成后，调用链固定为：

```text
Cognit / TurnService
→ CapabilityInvoker::invoke
→ AdmissionController::admit
→ ExecutionPermit
→ ToolRunner / Sandbox / External Runtime
→ UsageSettlement + AuditEvent
```

## 2. 核心不变量

1. 没有 `ExecutionPermit` 就不能执行副作用能力。
2. `SandboxFirst` 必须 fail closed：沙箱不可用、沙箱失败、沙箱结果无法验证时，真实执行不得继续。
3. Approval、Sandbox、Budget、Quota、Lease 是不同维度，不能互相替代。
4. Permit 必须绑定 `operation_id`、`process_id`、`capability`、`scope` 和有效期。
5. Usage 与 Audit 必须按 Operation 可追踪。
6. 新工具入口不得直接调用具体 Tool 实现，必须走 `CapabilityInvoker`。

## 3. 新增类型位置

```text
crates/fabric/src/types/admission.rs
crates/fabric/src/types/lease.rs
crates/fabric/src/include/admission.rs
crates/fabric/src/include/capability_invoker.rs
```

建议最小类型：

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PermitId(pub uuid::Uuid);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdmissionRequest {
    pub operation_id: OperationId,
    pub process_id: ProcessId,
    pub principal: PrincipalId,
    pub capability: CapabilityId,
    pub action: String,
    pub input_summary: String,
    pub risk: RiskLevel,
    pub requested_scope: CapabilityScope,
    pub budget: Option<BudgetRequest>,
    pub lease: Option<LeaseRequest>,
    pub sandbox: SandboxRequirement,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionPermit {
    pub id: PermitId,
    pub operation_id: OperationId,
    pub process_id: ProcessId,
    pub capability: CapabilityId,
    pub granted_scope: CapabilityScope,
    pub expires_at: MonoDeadline,
    pub sandbox: SandboxDecision,
    pub budget_reservation: Option<BudgetReservationId>,
    pub lease: Option<ResourceLeaseId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SandboxRequirement {
    NotRequired,
    Required,
    RequiredThenPromote,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AdmissionError {
    Denied { reason: String },
    ApprovalRequired { prompt: String },
    SandboxRequiredUnavailable,
    BudgetExceeded,
    QuotaExceeded,
    LeaseUnavailable,
    InvalidScope { reason: String },
}
```

## 4. Admission API

```rust
#[async_trait]
pub trait AdmissionController: Send + Sync {
    async fn admit(&self, request: AdmissionRequest)
        -> Result<ExecutionPermit, AdmissionError>;

    async fn settle(&self, permit: PermitId, usage: UsageReport)
        -> Result<(), AdmissionError>;

    async fn revoke(&self, permit: PermitId, reason: RevokeReason)
        -> Result<(), AdmissionError>;
}
```

`admit()` 只负责准入决策与资源预留，不执行 Tool。Tool 执行由 Capability 层完成。

## 5. CapabilityInvoker API

```rust
#[async_trait]
pub trait CapabilityInvoker: Send + Sync {
    async fn invoke(&self, request: CapabilityRequest) -> CapabilityResult;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityRequest {
    pub operation_id: OperationId,
    pub process_id: ProcessId,
    pub name: String,
    pub input: serde_json::Value,
    pub call_id: String,
    pub deadline: Option<MonoDeadline>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityResult {
    pub call_id: String,
    pub output: String,
    pub is_error: bool,
    pub usage: UsageReport,
    pub audit_id: Option<AuditEventId>,
}
```

第一阶段 `CapabilityInvoker` 内部可以继续适配现有 ToolRunner，但 ToolRunner 入口必须要求 `ExecutionPermit`。

## 6. SandboxFirst 修正

当前迁移目标：

```text
Verdict::SandboxFirst
→ AdmissionRequest.sandbox = RequiredThenPromote
→ Sandbox run
→ sandbox pass + verifier pass
→ real execution permit
```

禁止行为：

```text
SandboxFirst → 注入一段提示 → 继续正常执行
SandboxFirst → 日志警告“no sandbox” → 继续真实执行
```

缺少沙箱时返回：

```text
AdmissionError::SandboxRequiredUnavailable
```

并转换为用户可见、结构化的能力调用失败，而不是 panic 或静默降级。

## 7. 迁移步骤

### PR-5A：Admission 合约与 no-op 适配

- 新增 Admission 与 CapabilityInvoker contracts；
- 增加 `PermitId`、`CapabilityRequest`、`CapabilityResult`；
- 实现 `AllowAllAdmissionController` 仅用于测试，并在类型名中明确 `testing`；
- 生产路径不接入 AllowAll。

### PR-5B：ToolRunner 入口要求 Permit

- 修改 daemon tool executor，使工具执行函数参数包含 `ExecutionPermit`；
- `CapabilityInvoker` 负责从 ToolUse 构造 AdmissionRequest；
- 缺失 Permit 的 Tool 执行直接返回结构化错误。

### PR-5C：SandboxFirst fail closed

- 将 SelfField 的 `Verdict::SandboxFirst` 映射为 `SandboxRequirement::RequiredThenPromote`；
- 沙箱不可用时停止真实执行；
- 删除 prompt-only SandboxFirst 行为。

### PR-5D：Budget、Quota、Lease 原子预留

- 将预算扣减从执行后统计改为 admit 阶段预留、settle 阶段结算；
- Lease 到期或 revoke 时取消对应 Operation；
- Audit 记录 permit 生命周期。

## 8. 测试

```bash
cargo test -p fabric admission
cargo test -p executive capability_invoker
cargo test -p executive sandbox_first_fail_closed
cargo test -p executive budget_quota_lease
cargo check --workspace --all-targets
```

必须覆盖：

1. 没有 Permit 时 ToolRunner 拒绝执行；
2. SandboxFirst 且沙箱不可用时模型不能继续触发真实 Tool；
3. 沙箱失败时真实 Tool 调用次数为 0；
4. Approval 拒绝时不占用 Lease；
5. Budget 预留失败时无 Audit success；
6. Permit 超时后不能复用；
7. settle 只允许执行一次；
8. Operation cancel 会 revoke 仍活跃的 Permit。

## 9. 完成标准

- `rg "SandboxFirst.*Proceeding without sandbox|sandbox review" crates` 不再命中生产路径；
- `rg "execute_tool\(|ToolRunner" crates/executive` 显示所有副作用执行入口都需要 Permit；
- 所有 Capability 调用都有 OperationId 与 AuditEvent；
- Required sandbox 策略没有降级路径；
- 单元测试能证明预算、配额和 Lease 不会竞态超发。
