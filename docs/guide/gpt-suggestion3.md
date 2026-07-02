# Auro Memory Runtime

## Governed Memory for Persistent Intelligent Agents

**Version:** Draft v0.1
**Status:** Design Proposal

---

# 1. Why Memory Runtime

Auro Runtime 不应该只做短上下文对话。

真正长期运行的 Agent 必须拥有：

```text
可写入的长期记忆
可检索的历史经验
可更新的项目状态
可治理的用户偏好
可追踪的工作流经验
```

因此，Memory 不应该只是：

```text
Vector Database
RAG
Chat History
```

而应该是：

> **Memory Runtime**

也就是由 Runtime 管理的长期记忆系统。

---

# 2. Core Idea

Memory 不是数据库功能。

Memory 是 Runtime 的核心循环：

```text
Observe
  ↓
Extract
  ↓
Store
  ↓
Retrieve
  ↓
Inject
  ↓
Reason
  ↓
Update
```

一句话：

> **Memory Runtime 负责决定：什么值得记住，什么时候读取，如何注入上下文，如何更新、合并和删除。**

---

# 3. Memory Runtime Architecture

```text
User Message
    │
    ▼
Intent Classifier
    │
    ▼
Memory Retrieval Trigger
    │
    ▼
Memory Retriever
    │
    ▼
Context Builder
    │
    ▼
Provider / Native Agent
    │
    ▼
Response
    │
    ▼
Memory Write Detector
    │
    ▼
Memory Extractor
    │
    ▼
Memory Updater
    │
    ▼
Memory Store
```

---

# 4. Memory Types

Auro Runtime 应该区分不同类型的记忆。

## 4.1 User Memory

用户长期偏好和稳定背景。

例如：

```text
用户偏好 Rust
用户在机器人公司做 Linux 开发
用户希望项目偏系统架构
用户不想把 Robot 做成 Agent 本体
```

---

## 4.2 Project Memory

项目长期事实。

例如：

```text
Auro Runtime 是 Persistent Intelligent Runtime
项目当前约 20w Rust 代码
项目目标不是 Claude Wrapper
项目希望支持 Provider、Workflow、Plugin、Memory、Robot Capability
```

---

## 4.3 Workflow Memory

可复用流程。

例如：

```text
如何分析 ROS 仓库
如何排查机器人走路异常
如何管理 Claude Code SubAgent
如何生成开源文档
```

---

## 4.4 Robot Memory

机器人相关事实。

例如：

```text
机器人型号
URDF 路径
关节限制
控制频率
WBC 参数
MPC 配置
仿真环境
```

---

## 4.5 Trace Memory

历史执行轨迹。

例如：

```text
用户提出问题
Agent 生成计划
调用哪些工具
得到什么结果
最后如何回答
哪些步骤有效
哪些步骤失败
```

---

## 4.6 Knowledge Memory

长期知识。

例如：

```text
FK / IK / Jacobian
Dynamics
WBC
MPC
Active Inference
Global Workspace
Agent Runtime
```

---

# 5. Memory Schema

不要只存纯文本。

推荐结构化保存。

```json
{
  "id": "mem_001",
  "type": "project_goal",
  "subject": "Auro Runtime",
  "content": "Auro Runtime is designed as a persistent intelligent runtime, not a Claude wrapper.",
  "scope": "global",
  "confidence": 0.92,
  "source": "conversation",
  "created_at": "2026-07-01T10:00:00Z",
  "updated_at": "2026-07-01T10:00:00Z",
  "last_used_at": null,
  "tags": ["agent", "runtime", "provider"],
  "ttl": null,
  "status": "active"
}
```

---

# 6. Memory Scope

Memory 必须有作用域。

```text
global
project
session
workflow
robot
user
temporary
```

例如：

```text
global:
用户长期偏好

project:
Auro Runtime 架构设计

session:
当前对话临时上下文

workflow:
某个可复用流程

robot:
某台机器人配置

temporary:
短期缓存，过期删除
```

---

# 7. Write Trigger

不是所有内容都应该保存。

Memory Runtime 需要判断：

```text
这句话是否值得长期保存？
```

应该保存：

```text
长期偏好
稳定身份
长期项目
重要设计决策
反复出现的工作流
用户明确要求记住的内容
```

不应该保存：

```text
一次性问题
临时命令
短期情绪
无长期价值闲聊
敏感信息
低置信度猜测
```

---

# 8. Memory Candidate Detection

可以设计：

```rust
enum MemoryCandidate {
    UserPreference,
    UserProfile,
    ProjectFact,
    ProjectGoal,
    DesignDecision,
    WorkflowPattern,
    RobotFact,
    TemporaryFact,
    SensitiveFact,
    Ignore,
}
```

处理逻辑：

```text
UserPreference      → save
UserProfile         → save if stable
ProjectFact         → save
ProjectGoal         → save
DesignDecision      → save
WorkflowPattern     → save
RobotFact           → save
TemporaryFact       → short TTL
SensitiveFact       → ask / avoid
Ignore              → do nothing
```

---

# 9. Retrieval Trigger

Memory 读取不应该只靠关键词。

应该由任务意图触发。

例如：

```text
用户问：我的项目怎么开源？
触发：project_strategy

用户问：Robot Agent 架构怎么设计？
触发：agent_architecture + robot_runtime

用户问：这个仓库怎么分析？
触发：workflow_memory + project_memory
```

---

# 10. Retrieval Pipeline

```text
User Message
    ↓
Intent Detection
    ↓
Query Expansion
    ↓
Memory Search
    ↓
Ranking
    ↓
Deduplication
    ↓
Conflict Check
    ↓
Context Injection
```

Ranking 应考虑：

```text
语义相关性
时间新鲜度
置信度
是否被用户明确保存
是否最近使用
是否属于当前项目
```

---

# 11. Context Injection

不要把所有记忆塞进上下文。

应该分层注入。

## 11.1 Always Inject

极少量核心记忆。

```text
用户主要项目
长期技术方向
重要偏好
强约束
```

---

## 11.2 Task Relevant Inject

和当前任务相关的记忆。

```text
项目架构
历史设计决策
相关 workflow
相关 robot facts
```

---

## 11.3 Optional Inject

低优先级历史资料。

```text
历史对话片段
低置信度记忆
可选参考资料
```

---

# 12. Injection Format

建议使用结构化上下文。

```text
[User Memory]
- User works in a robotics company as a Linux developer.
- User prefers Rust and system-level architecture.

[Project Memory]
- Auro Runtime is a persistent intelligent runtime.
- Claude/Codex are providers or subagents, not the core.

[Design Constraints]
- Robot is a capability library, not the Agent itself.
- Runtime manages memory, workflow, state, lifecycle, and providers.
```

---

# 13. Memory Update

记忆需要演化。

必须支持：

```text
merge
update
delete
decay
conflict
pin
archive
```

---

## 13.1 Merge

相似记忆合并。

例如：

```text
Auro Runtime 不是 Claude Wrapper
Auro Runtime 不是 Claude 外接 Runtime
```

合并成：

```text
Auro Runtime treats Claude/Codex as replaceable providers or subagents, not as the core system.
```

---

## 13.2 Update

新记忆覆盖旧记忆。

例如：

```text
旧：项目叫 Robot Agent
新：项目叫 Auro Runtime
```

更新为：

```text
The project is now positioned as Auro Runtime.
```

---

## 13.3 Conflict

发现冲突时不要自动乱改。

应该询问用户：

```text
之前记录项目目标是 Robot Agent。
现在你说它是 Intelligent Runtime。
是否更新长期记忆？
```

---

## 13.4 Decay

长期不用的记忆降低权重。

```text
last_used_at 越久
retrieval_score 越低
```

---

## 13.5 Pin

用户明确要求保留的记忆，提高优先级。

```text
remember this
以后都按这个来
这是项目核心原则
```

---

# 14. Memory Deletion

用户应该可以管理记忆。

支持：

```bash
auro memory list
auro memory search "runtime"
auro memory show mem_001
auro memory forget mem_001
auro memory update mem_001
auro memory pin mem_001
```

这非常重要。

Memory 必须是：

```text
可解释
可查看
可删除
可更新
```

---

# 15. Storage Backend

不要一开始绑定某个数据库。

Memory Store 应该是接口。

```rust
trait MemoryStore {
    fn insert(&self, memory: Memory) -> Result<MemoryId>;
    fn update(&self, memory: Memory) -> Result<()>;
    fn delete(&self, id: MemoryId) -> Result<()>;
    fn get(&self, id: MemoryId) -> Result<Option<Memory>>;
    fn search(&self, query: MemoryQuery) -> Result<Vec<Memory>>;
}
```

可以支持：

```text
SQLite
Filesystem
Postgres
Qdrant
Milvus
Neo4j
Redis
```

---

# 16. Retrieval Interface

```rust
struct MemoryQuery {
    text: String,
    intent: Option<String>,
    scope: Option<MemoryScope>,
    tags: Vec<String>,
    limit: usize,
    min_confidence: f32,
}
```

返回：

```rust
struct RetrievedMemory {
    memory: Memory,
    score: f32,
    reason: String,
}
```

注意：

`reason` 很重要。

Agent 需要知道为什么这条记忆被取出。

---

# 17. Write Interface

```rust
struct MemoryWriteRequest {
    source_text: String,
    session_id: SessionId,
    scope: MemoryScope,
    candidate_type: MemoryCandidate,
    confidence: f32,
}
```

写入前应该经过：

```text
safety check
dedup check
merge check
user-control check
```

---

# 18. Trigger Design

Memory Runtime 至少需要四类触发器。

## 18.1 Explicit Trigger

用户明确要求：

```text
记住这个
以后按这个来
忘掉这个
更新一下
```

---

## 18.2 Implicit Trigger

用户表达了稳定信息：

```text
我现在在机器人公司做 Linux 开发
我项目现在已经 20w Rust 代码
我想做开源 Runtime
```

---

## 18.3 Task Trigger

任务需要相关记忆：

```text
继续上次那个方案
按我们之前讨论的来
把我的项目总结一下
```

---

## 18.4 Periodic Trigger

定期整理：

```text
合并重复记忆
归档过期记忆
总结项目状态
更新 workflow
```

---

# 19. Memory Safety

必须避免保存：

```text
敏感身份
隐私信息
临时位置
账号密码
API Key
医疗/政治/宗教等敏感属性
```

除非：

用户明确要求。

---

# 20. Memory and Self Evolution

Auro 的 Self Evolution 不应该被理解为模型参数自我更新。

而应该理解为：

```text
Memory Evolution
Workflow Evolution
Capability Evolution
Policy Evolution
Trace Evolution
```

也就是：

Runtime 通过经验不断改善行为。

完整闭环：

```text
Task
  ↓
Plan
  ↓
Act
  ↓
Observe
  ↓
Evaluate
  ↓
Remember
  ↓
Refine Workflow
  ↓
Next Task
```

---

# 21. MVP Plan

第一版只做五件事：

```text
1. Memory Schema
2. Memory Store: SQLite / JSON
3. Explicit Save / Forget
4. Task-Relevant Retrieval
5. Context Injection
```

不要一开始做复杂神经科学记忆模型。

---

# 22. CLI MVP

```bash
auro memory add "Auro Runtime treats Claude as provider, not core."
auro memory list
auro memory search "Claude provider"
auro memory forget mem_001
auro memory pin mem_002
```

对话中：

```text
User:
记住，Robot 只是 Capability Library，不是 Agent 本体。

Auro:
已保存为 Project Design Memory。
```

---

# 23. Future Direction

未来可以加入：

```text
Memory Graph
Episodic Memory
Semantic Memory
Procedural Memory
Emotional Tagging
Attention-based Retrieval
Active Inference Memory
Self Model
World Model
```

但是这些都应该作为：

```text
Cognition Plugin
Memory Backend
Retrieval Strategy
```

而不是写死在 Runtime Core。

---

# 24. Design Principle

```text
Memory is not RAG.

Memory is governed runtime state.

Memory must be explicit, inspectable, editable, scoped, ranked, and evolvable.
```

---

# 25. One Sentence

> **Auro Memory Runtime is a governed memory system for persistent agents: it decides what to remember, when to retrieve, how to inject context, and how to evolve memory through use.**
