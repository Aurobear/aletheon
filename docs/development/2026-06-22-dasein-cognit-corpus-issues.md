# dasein / cognit / corpus 当前问题分析

**日期:** 2026-06-22  
**范围:** 当前仓库整体可用性初查，重点分析：

- `crates/dasein`
- `crates/cognit`
- `crates/corpus`

**说明:** 本机未安装 Rust/Cargo，按要求不把“本机缺 cargo”作为项目问题处理；本文基于源码静态分析。所有代码判断尽量带文件行号锚点。

---

## 1. 总体结论

项目的架构意图清晰，已经形成三层核心模型：

```text
Runtime / daemon
      |
      +-- dasein  : SelfField，自我/边界/策略层，回答“该不该做”
      +-- cognit  : BrainCore，推理/规划/反思层，回答“怎么做”
      +-- corpus  : BodyRuntime，工具/执行/安全层，负责“实际执行”
```

但从当前实现看，项目还处在“架构骨架 + 局部能力实现 + 多处集成断点”的阶段。最主要的问题不是单个函数缺失，而是几个核心链路没有闭合：

1. **默认配置链路不稳**：`AppConfig::default()` 默认没有 provider，`load_layered()` 也不会读取仓库内 `config/default.toml`，裸环境下 daemon 很可能启动失败。
2. **corpus 工具执行链路存在设计级 bug**：`ToolRunnerWithGuard` 把所有 `L1+` 工具都当成 shell command sandbox 执行，导致 `file_write` / `apply_patch` / `web_fetch` 等非 command 型工具无法按自身逻辑运行。
3. **dasein 的 DaseinModule 默认没有真正接入 runtime**：runtime 创建 `SelfField` 时未启用 `enable_dasein`，perception injection 也被丢弃。
4. **cognit 的推理/反思实现仍偏模板化**：LLM 反思文本生成后被丢弃，普通 `think()` 不解析多步计划，复杂规划链路不完整。
5. **测试/CI 可见代码存在编译风险**：`ProviderConfig` 字段变更后多个测试构造未同步；`Observation`、`async_trait` 等测试导入可见缺失。

---

## 2. 可用性判断

### 2.1 裸环境启动风险：默认 provider 为空

`AppConfig::default()` 明确将 `providers` 初始化为空：

- `crates/cognit/src/config/mod.rs:430-435`

`load_layered()` 的逻辑只加载用户全局配置 `~/.aletheon/config.toml` 和项目本地 `.aletheon/config.toml`，没有读取仓库里的 `config/default.toml`：

- `crates/cognit/src/config/mod.rs:399-426`

daemon 启动时使用该配置创建 registry，并立即解析默认 provider/model：

- `crates/runtime/src/impl/daemon/mod.rs:100-113`

如果用户没有先运行 `setup.sh` 生成 `~/.aletheon/config.toml`，`registry.resolve("")` 很可能因为没有默认 provider 而失败。

`setup.sh` 确实会写入一份带 providers 的用户配置：

- `setup.sh:289-334`

但这意味着当前项目依赖安装脚本来补足可运行配置，而不是代码默认值本身可用。

**影响:** 用户直接从源码运行 daemon / exec，可能无法启动。  
**建议:** 二选一：

- 让 `AppConfig::default()` 内置一组最小默认 provider 配置；或
- 让 `load_layered()` 显式读取仓库 `config/default.toml` 作为 Layer 0。

---

### 2.2 exec 模式和 daemon 模式行为不一致

`aletheon-exec` 单独创建 provider、tool registry、runner，然后自己循环调用 LLM 和工具：

- `crates/runtime/src/bin/aletheon-exec.rs:141-160`
- `crates/runtime/src/bin/aletheon-exec.rs:194-245`

这条路径绕开了 daemon handler 中的较多 runtime 设施。daemon 的 chat handler 还会创建 event sink、注入 dasein context、处理 approval/event 等：

- `crates/runtime/src/impl/daemon/handler/chat.rs:439-455`

**影响:** 用户通过 exec 和 daemon 使用同一个项目，可能得到完全不同的 agent 行为。  
**建议:** 把 exec 改成复用 daemon 的核心 turn pipeline，或明确标注 exec 只是简化调试入口。

---

## 3. dasein 问题分析

### 3.1 模块责任

`dasein` 的公开入口说明它是 SelfField / policy engine：

- `crates/dasein/src/lib.rs:1-18`
- `crates/dasein/src/lib.rs:38-51`

核心 `SelfField` 组合了边界、身份、关切、叙事、冲突、注意力、连续性、变更审查和多个桥接层：

- `crates/dasein/src/core/mod.rs:83-103`

`review()` 是主要决策入口，顺序为 Hook -> Policy -> Boundary -> Care -> Permission -> Narrative -> Attention：

- `crates/dasein/src/core/mod.rs:343-421`

### 3.2 DaseinModule 默认未启用

`SelfFieldConfig` 虽然提供了 `enable_dasein` 和 Dasein 参数：

- `crates/dasein/src/core/mod.rs:46-63`

但默认值是关闭：

- `crates/dasein/src/core/mod.rs:65-79`

runtime 创建 SelfField 时只设置了 `db_path`，没有设置 `enable_dasein: true`：

- `crates/runtime/src/impl/daemon/handler/mod.rs:363-368`

而 Dasein prompt 注入依赖 `sf.dasein_prompt_injection()` 返回 Some：

- `crates/runtime/src/impl/daemon/handler/chat.rs:443-451`

因此当前 daemon 默认不会真正获得 Dasein existential context。

**影响:** README/架构中强调的 self-awareness / Dasein 体验，在实际默认 runtime 中基本不生效。  
**建议:** 明确产品决策：

- 如果 Dasein 是核心特性：runtime 应设置 `enable_dasein: true`，并提供配置开关；
- 如果 Dasein 是实验特性：文档应标注默认关闭，避免用户误判。

---

### 3.3 PerceptionManager 启动了，但注入没有被消费

daemon 启动 perception manager 和 bridge：

- `crates/runtime/src/impl/daemon/mod.rs:231-260`

`PerceptionBridge` 能把高优先级事件转为 `PerceptionInjection::Immediate`，低优先级事件批量发送：

- `crates/dasein/src/impl/perception/bridge.rs:17-24`
- `crates/dasein/src/impl/perception/bridge.rs:63-97`

但 `RequestHandler::new()` 接收的参数命名为 `_perception_rx`，函数体里未使用：

- `crates/runtime/src/impl/daemon/handler/mod.rs:195-200`

**影响:** 系统 perception 事件虽然被采集和桥接，但不会影响对话、计划或 Dasein 状态。  
**建议:** 在 handler 中保存 `perception_rx`，在 turn 开始/结束时 drain：

```text
PerceptionManager -> PerceptionBridge -> RequestHandler.perception_rx
                                      -> prompt injection / world model update / dasein event
```

---

### 3.4 DaseinEventBridge 可能被调用但无实际效果

SelfField 提供 `wire_dasein_event_bridge()`：

- `crates/dasein/src/core/mod.rs:237-252`

它只有在 `self.dasein` 和 `dasein_event_tx` 同时存在时才订阅 EventBus：

- `crates/dasein/src/core/mod.rs:246-249`

runtime 确实尝试调用它：

- `crates/runtime/src/impl/daemon/handler/mod.rs:370-374`

但由于 SelfField 默认未启用 Dasein，调用通常是 no-op。

**影响:** EventBus -> DaseinModule 的设计链路看起来存在，实际默认不连通。  
**建议:** 要么启用 DaseinModule，要么在启动日志中明确说明 Dasein disabled，避免误以为已连接。

---

### 3.5 SelfFieldStore 初始化错误被静默吞掉

`SelfField::new()` 中 store 初始化：

- `crates/dasein/src/core/mod.rs:123-125`

代码用 `.and_then(|path| SelfFieldStore::new(path).ok())`，如果 SQLite store 创建失败，会直接变成 `None`。

**影响:** 持久化失败不会阻止启动，也没有 warning，用户会误以为 narrative / attention / care 等状态已持久化。  
**建议:** 至少记录 `warn!`；更稳妥是让 `SelfField::new()` 返回 `Result<Self>`，把持久化失败显式暴露给上层。

---

### 3.6 SorgeLoop 生命周期只能启动一次

`SorgeLoop::start()` 从 `event_rx: Mutex<Option<Receiver>>` 中 `take()` receiver：

- `crates/dasein/src/dasein/sorge.rs:49-50`

`stop()` 只是把 `running` 置 false：

- `crates/dasein/src/dasein/sorge.rs:175-177`

停止后 receiver 不会放回 Option，因此再次 `start()` 会返回 `None`。

**影响:** 如果 subsystem 需要 restart、reload、health recovery，Dasein loop 无法重新启动。  
**建议:** 保存 JoinHandle 并在 stop 时等待退出；restart 时重建 channel 或重建 SorgeLoop。

---

## 4. cognit 问题分析

### 4.1 模块责任

`cognit` 的公开入口说明 BrainCore 负责 reasoning、planning、reflection、learning，不负责“should I”：

- `crates/cognit/src/lib.rs:1-17`
- `crates/cognit/src/lib.rs:24-43`

`BrainCore` 聚合 reasoner、planner、critic、reflector、learner、world model、skill extractor、LLM/dual model/learning bridge：

- `crates/cognit/src/core/mod.rs:70-88`

`BrainCoreOps::think()` 是主推理入口：

- `crates/cognit/src/core/brain_core_ops.rs:20-26`

### 4.2 测试代码和 ProviderConfig 字段不同步

`ProviderConfig` 现在包含 `max_context_length: Option<usize>`：

- `crates/cognit/src/config/mod.rs:133-145`

但 `provider_factory` 测试中多个 `ProviderConfig { ... }` 没有设置该字段，例如：

- `crates/cognit/src/impl/llm/provider_factory.rs:135-141`
- `crates/cognit/src/impl/llm/provider_factory.rs:149-155`
- `crates/cognit/src/impl/llm/provider_factory.rs:163-169`

`scheduler` 中的正式代码已补 `max_context_length: None`：

- `crates/cognit/src/impl/llm/scheduler.rs:69-80`

**影响:** `cargo test --workspace` 很可能在 cognit 相关测试编译阶段失败。  
**建议:** 给所有测试构造补 `max_context_length: None`。

---

### 4.3 core tests 缺少必要导入

`crates/cognit/src/core/tests.rs` 顶部导入了 `ExecutionResult, Experience, ReflectionEntry, ReflectionOutcome`，但后续直接使用 `Observation`：

- `crates/cognit/src/core/tests.rs:1-8`
- `crates/cognit/src/core/tests.rs:42-46`
- `crates/cognit/src/core/tests.rs:107-111`

同一文件还使用 `#[async_trait]`：

- `crates/cognit/src/core/tests.rs:335-336`

但顶部未看到 `use async_trait::async_trait;`。

**影响:** 测试编译风险。  
**建议:** 修改为：

```rust
use async_trait::async_trait;
use base::brain::{ExecutionResult, Experience, Observation, ReflectionEntry, ReflectionOutcome};
```

---

### 4.4 普通 think() 不解析 LLM 多步计划

`think()` 单 LLM 路径调用 LLM 后，直接把完整 response 作为 reasoning 交给 `planner.generate_plan()`：

- `crates/cognit/src/core/brain_core_ops.rs:101-137`

`Planner::generate_plan()` 固定只生成一个 `PlanStep`：

- `crates/cognit/src/core/planner.rs:24-49`

而多步解析逻辑只在 `think_with_refinement()` 的 `generate_initial_plan()` 中使用：

- `crates/cognit/src/core/mod.rs:328-390`

**影响:** 即使 LLM 输出 JSON subtasks，普通 `BrainCoreOps::think()` 仍只产生单步计划。复杂任务规划能力被弱化。  
**建议:** 把 `parse_subtasks()` 合并进 `think()` 的单 LLM 和 dual-model executor 路径，或者让 runtime 默认使用 `think_with_refinement()`。

---

### 4.5 LLM 反思结果被丢弃

`reflect()` 在存在 LLM 时会构造 prompt 并调用 LLM：

- `crates/cognit/src/core/brain_core_ops.rs:150-171`

但 LLM 文本只放入 `_analysis`，后续仍返回模板 reflector 的结果：

- `crates/cognit/src/core/brain_core_ops.rs:172-184`

**影响:** 付出了 LLM 调用成本，但结构化 Reflection 没有利用 LLM 输出。用户看到的反思仍主要是规则模板。  
**建议:** 增加结构化解析，例如要求 LLM 输出 JSON：`what_worked`, `what_failed`, `what_to_improve`, `confidence`；解析失败再 fallback 到模板 reflector。

---

### 4.6 API key 缺失静默变成空字符串

`ProviderRegistry::resolve_api_key()` 找不到配置和环境变量时返回空字符串：

- `crates/cognit/src/impl/provider_registry.rs:158-165`

`provider_factory::resolve_api_key()` 同样返回空字符串：

- `crates/cognit/src/impl/llm/provider_factory.rs:100-107`

**影响:** provider 创建阶段不会失败，错误会延迟到首次 API 请求，通常表现为 401 或 provider 协议错误，不利于排查。  
**建议:** 对需要 key 的 provider 在创建时 fail fast；对 Ollama 这类本地 provider 允许空 key。

---

### 4.7 provider auto-detection 规则过窄

`ProviderRegistry::detect_transport()` 只在 URL 以 `/anthropic` 结尾时识别 Anthropic：

- `crates/cognit/src/impl/provider_registry.rs:15-27`

`provider_factory::detect_provider_kind()` 也使用类似规则，并额外识别 Ollama：

- `crates/cognit/src/impl/llm/provider_factory.rs:11-26`

**影响:** `https://api.anthropic.com` 如果 transport 写成 `auto`，会被识别成 OpenAI。  
**建议:** 使用 URL host 判断：host 包含 `anthropic.com` 时识别为 Anthropic；或要求 Anthropic provider 必须显式配置 `transport = "anthropic"` 并在配置校验阶段报错。

---

## 5. corpus 问题分析

### 5.1 模块责任

`corpus` 是执行身体，公开 re-export core、drivers、tools、security：

- `crates/corpus/src/lib.rs:1-15`

`AletheonBodyRuntime` 包含工具 registry、guarded runner、capabilities 和初始化状态：

- `crates/corpus/src/core/mod.rs:15-21`

默认工具在 `ToolRegistry::default()` 里注册：

- `crates/corpus/src/tools/tools/registry.rs:86-161`

`BodyRuntime::execute()` 流程是找工具、转换 context、通过 `ToolRunnerWithGuard` 执行：

- `crates/corpus/src/core/mod.rs:113-154`

---

### 5.2 P0：L1+ 非 command 工具不会按自身逻辑执行

`ToolRunnerWithGuard::execute_tool()` 对 `PermissionLevel::L1` 及以上工具走 sandbox：

- `crates/corpus/src/security/security/runner.rs:296-329`

但它取 command 的方式是固定读取 `input.get("command")`：

- `crates/corpus/src/security/security/runner.rs:297-303`

这只适合 `bash_exec`，因为 `bash_exec` schema 有 `command`：

- `crates/corpus/src/tools/tools/bash_exec.rs:20-35`

但 `file_write` schema 是 `path` + `content`：

- `crates/corpus/src/tools/tools/file_write.rs:19-33`

`file_write` 的真实写文件逻辑在自己的 `execute()` 里：

- `crates/corpus/src/tools/tools/file_write.rs:44-88`

当前 guarded path 对 `file_write` 会执行空 command 或 sandbox 空命令，而不是调用 `FileWriteTool::execute()`。

**影响:** agent 的写文件、patch、web fetch/search 等 L1 工具可能整体不可用或行为异常。  
**建议:** 改成按工具类型区分：

```text
bash_exec             -> command sandbox backend
file_write/apply_patch -> tool.execute() + path policy/sandbox profile
web_fetch/web_search   -> tool.execute() + network policy/approval
module/kernel tools    -> explicit approval + dedicated executor
```

---

### 5.3 `Action.requires_sandbox` / `Action.timeout` 未贯穿执行

`Action` 结构本身定义了：

- `requires_sandbox`: `crates/base/src/include/body.rs:19-25`
- `timeout`: `crates/base/src/include/body.rs:26-27`

但 `AletheonBodyRuntime::execute()` 只把 `action.parameters` 传给 runner：

- `crates/corpus/src/core/mod.rs:115-132`

runner 内部又对 L1+ 固定使用 30 秒 sandbox timeout：

- `crates/corpus/src/security/security/runner.rs:300-308`

**影响:** planner 生成的 action 约束无法真正影响执行层。  
**建议:** 把 `Action` 或 action metadata 传给 runner，让 runner 尊重 `requires_sandbox` 和 `timeout`。

---

### 5.4 policy 权限推断和真实工具权限脱节

`PolicyEngine` 有一个硬编码 `infer_tool_level()`：

- `crates/corpus/src/security/security/policy.rs:102-109`

但真实权限在每个工具自己的 `permission_level()` 中，例如：

- `bash_exec` 是 L1：`crates/corpus/src/tools/tools/bash_exec.rs:38-40`
- `file_write` 是 L1：`crates/corpus/src/tools/tools/file_write.rs:36-38`
- `kernel_build` 是 L3：`crates/corpus/src/tools/tools/kernel_build.rs:60-62`

**影响:** 新工具或高危工具如果没加入 `infer_tool_level()`，policy 层可能低估风险。虽然 runner 后续也拿真实 `tool.permission_level()` 做 approval/audit，但 policy 的第一层判断并不可靠。  
**建议:** 删除 `infer_tool_level()`，让 `PolicyEngine::check()` 接收真实 permission level，或把 policy 绑定到 registry definitions。

---

### 5.5 sandbox auto 可能静默降级

`SandboxExecutor::select_backend()` 在 `Auto` 或 `BestEffort` 下选择第一个可用 backend：

- `crates/corpus/src/security/sandbox/executor.rs:67-75`

只有 `BestEffort` 且降级到 `IsolationLevel::None` 时才 warn：

- `crates/corpus/src/security/sandbox/executor.rs:110-114`

**影响:** `Auto` 模式下如果没有 bubblewrap，只剩 process/noop backend，用户可能不知道隔离强度已下降。  
**建议:** sandbox backend 选择结果写入 audit，并在 `Auto -> Noop` 时至少 warn。

---

### 5.6 audit session_id 为空

`ToolRunnerWithGuard::log_audit()` 中 `session_id` 被写成空字符串：

- `crates/corpus/src/security/security/runner.rs:416-430`

**影响:** 多 session 下 audit 记录无法按 session 追踪。  
**建议:** 从 `ToolContext.session_id` 或上层 Context 传入 log_audit。

---

### 5.7 FileRead / FileWrite 缺少路径边界约束

`file_read` 对绝对路径直接读取：

- `crates/corpus/src/tools/tools/file_read.rs:48-57`

`file_write` 对绝对路径直接写入，并会创建父目录：

- `crates/corpus/src/tools/tools/file_write.rs:44-58`

当前路径边界依赖外层 policy/sandbox，但如上所述，非 command 型 L1 工具的 sandbox 执行模型本身有问题。

**影响:** 一旦这些工具绕过或被直接调用，文件读写边界不清晰。  
**建议:** 在工具自身也做路径 canonicalize + workspace/writable root 校验，不只依赖 runner。

---

## 6. 附带发现

### 6.1 config/default.toml 与实际 compiled defaults 不一致

仓库 `config/default.toml` 看起来是完整配置参考，包含 providers：

- `config/default.toml:23-41`

但 `AppConfig::default()` 不包含这些 providers：

- `crates/cognit/src/config/mod.rs:430-435`

`config/default.toml` 里 agent loop 设置 `max_tool_calls = 0` 并注释 0 为 unlimited：

- `config/default.toml:98-101`

需要确认 runtime 实现是否也按 unlimited 解释。若实现按普通计数判断，0 会导致禁用工具调用。

---

### 6.2 README 链接需要持续校验

README 指向架构文档：

- `README.md:176`

当前仓库存在 `docs/arch.md`，但该文件曾在工作区状态里出现删除痕迹；后续提交前应确认它不被误删。

---

## 7. 建议修复优先级

### P0：先让基本 agent 可用

1. 修 `corpus::ToolRunnerWithGuard` 的 L1+ 工具执行逻辑。
2. 统一默认配置加载，保证无用户配置时也能给出清晰错误或可启动默认配置。
3. 修 cognit 测试构造和导入问题，保证 CI 能跑。

### P1：打通三层核心链路

1. 决定 Dasein 是否默认启用，并让 runtime 配置可控。
2. 消费 `PerceptionInjection`，把 perception 事件注入 prompt / world model / DaseinEvent。
3. 让 BrainCore 普通 `think()` 支持多步计划解析。
4. 让 LLM reflection 输出真正进入 `Reflection`。

### P2：安全与可观测性

1. API key 缺失 fail fast。
2. sandbox 降级写日志和 audit。
3. audit 记录真实 session_id。
4. FileRead/FileWrite 自身加路径边界校验。
5. SelfFieldStore 初始化失败显式 warning 或返回错误。

---

## 8. 推荐阅读路径

如果要继续深入排查，建议按以下顺序读：

```text
1. 配置与启动
   crates/cognit/src/config/mod.rs
   crates/runtime/src/impl/daemon/mod.rs
   setup.sh

2. daemon turn pipeline
   crates/runtime/src/impl/daemon/handler/mod.rs
   crates/runtime/src/impl/daemon/handler/chat.rs

3. corpus 执行层
   crates/corpus/src/core/mod.rs
   crates/corpus/src/security/security/runner.rs
   crates/corpus/src/tools/tools/registry.rs
   crates/corpus/src/tools/tools/bash_exec.rs
   crates/corpus/src/tools/tools/file_write.rs

4. cognit 认知层
   crates/cognit/src/core/brain_core_ops.rs
   crates/cognit/src/core/mod.rs
   crates/cognit/src/core/planner.rs
   crates/cognit/src/core/reflector.rs

5. dasein 自我层
   crates/dasein/src/core/mod.rs
   crates/dasein/src/dasein/mod.rs
   crates/dasein/src/dasein/sorge.rs
   crates/dasein/src/impl/perception/bridge.rs
```

---

## 9. 一句话总结

当前项目最需要优先处理的是：**把“配置 -> LLM -> 规划 -> 工具执行 -> perception/self-field 反馈”这条闭环打通**。现在三层模块都存在，但默认配置、Dasein 启用、perception 注入、corpus 非 bash 工具执行这几个关键接口还没有完全闭合。
