# Phase 3：Context Space 与事务化 Agora

## 1. 边界

```text
Context Space：决定某个 Process 能看到什么
Agora：保存经过提交的共享工作状态
Mnemosyne：保存长期经验和事实
Session：保存对话连续性
```

这四者不能继续互相替代。

## 2. Context Space 类型

新增：

```text
crates/fabric/src/types/space.rs
crates/fabric/src/include/space.rs
crates/executive/src/kernel/space/
```

```rust
pub struct ContextSpace {
    pub id: SpaceId,
    pub owner: ProcessId,
    pub parent_snapshot: Option<SpaceSnapshotId>,
    pub bindings: Vec<ContextBinding>,
    pub overlay: VersionedOverlay,
    pub namespace: NamespaceId,
}

pub enum ContextBinding {
    Session(SessionId),
    Agora(AgoraSpaceId, AgoraVersion),
    MemoryView(MemoryViewId),
    Artifact(ArtifactId, AccessMode),
    WorldProjection(WorldProjectionId, ProjectionVersion),
}
```

Space 只保存引用和版本，不保存全部文本副本。

## 3. Fork

```rust
async fn fork_space(parent: SpaceId, owner: ProcessId) -> Result<SpaceId>;
```

语义：

1. 固定父 bindings 的版本；
2. 创建空 private overlay；
3. 默认不继承写权限；
4. 子空间修改不影响父空间；
5. 显式 proposal 后才能共享。

## 4. Agora 数据模型

对现有 `Workspace` 增加：

```rust
pub struct Workspace {
    pub id: AgoraSpaceId,
    pub version: u64,
    pub blackboard: Blackboard,
    pub attention: Attention,
    pub task_graph: TaskGraph,
    pub trace: Trace,
    pub claims: ClaimTable,
}
```

提案：

```rust
pub struct AgoraProposal {
    pub id: ProposalId,
    pub space: AgoraSpaceId,
    pub author: ProcessId,
    pub base_version: u64,
    pub operation: AgoraOperation,
    pub evidence: Vec<EvidenceRef>,
    pub confidence: f32,
    pub ttl: Option<Duration>,
}
```

## 5. API

```rust
#[async_trait]
pub trait AgoraService: Send + Sync {
    async fn view(&self, req: AgoraViewRequest) -> Result<AgoraView>;
    async fn propose(&self, proposal: AgoraProposal) -> Result<ProposalId>;
    async fn commit(&self, id: ProposalId, permit: CommitPermit)
        -> Result<CommitReceipt>;
    async fn reject(&self, id: ProposalId, reason: RejectReason) -> Result<()>;
    async fn changes_since(&self, space: AgoraSpaceId, version: u64)
        -> Result<Vec<AgoraCommit>>;
}
```

原有 `publish/update` 只作为 `AgoraRegistry` backend 内部方法，不再作为公共主接口。

## 6. 并发与冲突

第一阶段在单进程 `Mutex<Workspace>` 下执行 compare-and-swap：

```text
proposal.base_version == workspace.version
→ apply typed operation
→ version += 1
→ append commit

否则：Conflict { expected, actual }
```

不做 CRDT 和分布式一致性。

## 7. Tool Evidence 流程

当前 ToolResult 直接记录到 Agora trace。改为：

```text
ToolResult
→ private Operation trace
→ Evidence object
→ Cognit/Reviewer decides relevance
→ AgoraProposal::AcceptEvidence
→ commit
```

Tool 成功不等于结果是全局事实。

## 8. 持久化

不要每轮把完整 Agora snapshot 作为字符串写入 RecallMemory。

改为：

- append-only `AgoraCommit` 写入 Mnemosyne；
- 每 N 次 commit 生成 checkpoint；
- 恢复 = checkpoint + commits；
- Session close 时明确 clear ephemeral workspace。

## 9. 迁移步骤

1. 给 `Workspace` 增加 version 和 commit log；
2. 新增 Proposal API，并用现有 Registry 实现；
3. 把 `turn_input` 移到 private Context overlay；
4. 把 Tool Evidence 改为 proposal；
5. 替换 `commit_agora_snapshot`；
6. 将 Attention/TaskGraph 接入 `AgoraView` 和 Harness。

## 10. 测试

```bash
cargo test -p agora
cargo test -p executive context_space
cargo test -p executive agora_integration
```

必须覆盖：

- fork 后子修改不污染父空间；
- base version 冲突；
- 未授权 commit 被拒绝；
- TTL entry 过期；
- commit replay 恢复同一状态；
- Tool Evidence 未经 commit 不出现在共享 view。

