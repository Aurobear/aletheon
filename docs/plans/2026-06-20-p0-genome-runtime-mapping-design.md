# P0 残留: Genome → Runtime 行为映射 — 实现计划

> **For agentic workers:** Use `workflow-feature` or `writing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让基因组中的 `ReasoningConfig` 和 `CareExt` 实际影响运行时行为。当 morphogenesis pipeline 变异基因组后，runtime 的推理策略和 care 权重应该随之改变。

**Architecture:** `AletheonRuntime` 持有一个 `GenomeConfig`（从 GenomeMeta 提取的轻量配置快照），在 genome 变异后通过 `EvolutionCoordinator` 的回调更新。`Reasoner` 从 config 读取策略，care 权重注入 system prompt。

**Tech Stack:** Rust, existing aletheon-* crates

---

## 当前状态

| 组件 | 状态 | 问题 |
|---|---|---|
| `GenomeMeta.reasoning` | ✅ 存在 | ReasoningConfig 有 default_strategy, reflection_trigger, impasse_threshold |
| `GenomeMeta.care_ext` | ✅ 存在 | CareExt 有 weights (HashMap<String, f64>) 和 boundary_rules |
| `Reasoner::think()` | ⚠️ 不读 config | 策略从构造函数传入，运行时不能改 |
| `AletheonRuntime` | ❌ 无 genome 引用 | 不知道基因组的任何配置 |
| `RuntimeConfig` | ❌ 无 genome 字段 | 只有通用 session/iteration 字段 |
| EvolutionCoordinator | ❌ 不回调 runtime | pipeline 变异后不更新 runtime 行为 |

## 目标数据流

```
GenomeMeta (磁盘)
    ↓ load
AletheonRuntime.genome_config (内存快照)
    ├── reasoning_strategy → Reasoner.set_default_strategy()
    ├── care_weights → system prompt injection
    └── impasse_threshold → StormBreaker trigger

MorphogenesisPipeline.migrate()
    ↓ callback
AletheonRuntime.update_genome_config()
    ├── 更新 reasoning_strategy
    └── 更新 care_weights
```

---

## 文件变更

| Action | File | Purpose |
|---|---|---|
| Modify | `crates/aletheon-runtime/src/core/config.rs` | 新增 GenomeConfig 结构体 |
| Modify | `crates/aletheon-runtime/src/core/orchestrator.rs` | AletheonRuntime 持有 GenomeConfig，传递给 Reasoner |
| Modify | `crates/aletheon-runtime/src/core/evolution_coordinator.rs` | pipeline 成功后回调更新 GenomeConfig |
| Modify | `crates/aletheon-brain/src/core/reasoner.rs` | think() 接受可选 care weights，注入推理链 |
| Create | `crates/aletheon-runtime/tests/genome_runtime_mapping.rs` | 集成测试 |

---

## Task 1: 新增 GenomeConfig

**Files:** `crates/aletheon-runtime/src/core/config.rs`

在 RuntimeConfig 下方新增：

```rust
use std::collections::HashMap;

/// Lightweight genome config snapshot held by the runtime.
/// Extracted from GenomeMeta — does not hold the full genome.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenomeConfig {
    /// Reasoning strategy name (e.g., "plan-then-execute", "react").
    pub reasoning_strategy: String,
    /// Confidence threshold below which the agent considers itself stuck.
    pub impasse_threshold: f64,
    /// What triggers reflection.
    pub reflection_trigger: String,
    /// Care weights by topic (e.g., "safety" -> 1.0).
    pub care_weights: HashMap<String, f64>,
    /// Current genome version string.
    pub genome_version: String,
}

impl Default for GenomeConfig {
    fn default() -> Self {
        Self {
            reasoning_strategy: "plan-then-execute".to_string(),
            impasse_threshold: 0.3,
            reflection_trigger: "task_complete".to_string(),
            care_weights: HashMap::new(),
            genome_version: "0.1.0".to_string(),
        }
    }
}

impl GenomeConfig {
    /// Extract from a GenomeMeta.
    pub fn from_genome_meta(meta: &aletheon_meta::GenomeMeta) -> Self {
        Self {
            reasoning_strategy: meta.reasoning.default_strategy.clone(),
            impasse_threshold: meta.reasoning.impasse_threshold,
            reflection_trigger: meta.reasoning.reflection_trigger.clone(),
            care_weights: meta.care_ext.weights.clone(),
            genome_version: meta.genome_version.clone(),
        }
    }

    /// Format care weights for injection into system prompt.
    pub fn care_weights_prompt(&self) -> String {
        if self.care_weights.is_empty() {
            return String::new();
        }
        let mut parts: Vec<String> = self.care_weights.iter()
            .map(|(k, v)| format!("  {}: {:.2}", k, v))
            .collect();
        parts.sort();
        format!("Current care priorities:\n{}", parts.join("\n"))
    }

    /// Map strategy name to ReasoningStrategy enum.
    pub fn reasoning_strategy_enum(&self) -> ReasoningStrategy {
        match self.reasoning_strategy.as_str() {
            "react" | "direct" => ReasoningStrategy::Direct,
            "chain-of-thought" | "plan-then-execute" => ReasoningStrategy::ChainOfThought,
            _ => ReasoningStrategy::ChainOfThought,
        }
    }
}
```

Add import: `use crate::core::react_loop::ReasoningStrategy;` — or re-export ReasoningStrategy from react_loop if it's defined there. If ReasoningStrategy is in brain crate, use `use aletheon_brain::core::reasoner::ReasoningStrategy;`.

### Tests

```rust
#[test]
fn test_care_weights_prompt_empty() {
    let config = GenomeConfig::default();
    assert_eq!(config.care_weights_prompt(), "");
}

#[test]
fn test_care_weights_prompt_with_values() {
    let mut config = GenomeConfig::default();
    config.care_weights.insert("safety".to_string(), 1.0);
    config.care_weights.insert("helpfulness".to_string(), 0.8);
    let prompt = config.care_weights_prompt();
    assert!(prompt.contains("safety: 1.00"));
    assert!(prompt.contains("helpfulness: 0.80"));
}

#[test]
fn test_strategy_mapping() {
    let mut config = GenomeConfig::default();
    config.reasoning_strategy = "direct".to_string();
    assert_eq!(config.reasoning_strategy_enum(), ReasoningStrategy::Direct);
    config.reasoning_strategy = "chain-of-thought".to_string();
    assert_eq!(config.reasoning_strategy_enum(), ReasoningStrategy::ChainOfThought);
}
```

---

## Task 2: AletheonRuntime 持有 GenomeConfig

**Files:** `crates/aletheon-runtime/src/core/orchestrator.rs`

### Step 1: Add GenomeConfig field

```rust
use crate::core::config::GenomeConfig;

pub struct AletheonRuntime {
    config: RuntimeConfig,
    react_loop: ReActLoop,
    evolution: Option<EvolutionCoordinator>,
    genome_config: GenomeConfig,
}
```

### Step 2: Update constructors

```rust
pub fn new(config: RuntimeConfig) -> Self {
    let react_loop = ReActLoop::new(config.clone());
    Self {
        config,
        react_loop,
        evolution: None,
        genome_config: GenomeConfig::default(),
    }
}

/// Set genome config (e.g., after loading genome from disk).
pub fn with_genome_config(mut self, genome_config: GenomeConfig) -> Self {
    self.genome_config = genome_config;
    self
}
```

### Step 3: Add accessor and updater

```rust
/// Get the current genome config.
pub fn genome_config(&self) -> &GenomeConfig {
    &self.genome_config
}

/// Update genome config (called after genome mutation).
pub fn update_genome_config(&mut self, genome_config: GenomeConfig) {
    self.genome_config = genome_config;
}
```

### Step 4: Inject care weights into system prompt

In `process_react()`, before calling `react_loop.run()`, compose the system prompt with care weights:

```rust
pub async fn process_react<L, R, F, Fut>(
    &mut self,
    input: &str,
    ctx: &Context,
    review_fn: R,
    llm: &L,
    tool_defs: &[ToolDefinition],
    execute_tool: F,
) -> Result<String>
where
    L: LlmProvider + ?Sized,
    R: Fn(&Intent, &Context) -> Result<Verdict>,
    F: Fn(&str, &str, &serde_json::Value) -> Fut,
    Fut: Future<Output = (String, bool)>,
{
    // Inject care weights into system prompt
    let care_prompt = self.genome_config.care_weights_prompt();
    if !care_prompt.is_empty() {
        let current_prompt = self.react_loop.system_prompt().to_string();
        let enhanced = format!("{}\n\n{}", current_prompt, care_prompt);
        self.react_loop.set_system_prompt(enhanced);
    }

    // ... existing logic
}
```

---

## Task 3: EvolutionCoordinator 回调更新 GenomeConfig

**Files:** `crates/aletheon-runtime/src/core/evolution_coordinator.rs`

### Step 1: Add genome_config update to post_turn

After the morphogenesis pipeline runs successfully, update the genome config:

In `EvolutionCoordinator`, add a method to extract updated config from pipeline results:

```rust
/// Extract updated GenomeConfig from pipeline results.
/// Returns Some if any migration succeeded.
pub fn extract_genome_update(results: &[PipelineResult]) -> Option<GenomeConfig> {
    for result in results {
        if result.success {
            if let Some(ref migration) = result.migration {
                // The pipeline migrated — we need the new genome config.
                // For now, we reconstruct from the candidate's genome.
                if let Some(ref candidate) = result.candidate {
                    // TODO: Extract GenomeMeta from candidate and build GenomeConfig
                    // This requires the candidate to carry GenomeMeta, not just Genome
                }
            }
        }
    }
    None
}
```

Actually, the cleaner approach: EvolutionCoordinator stores the current GenomeConfig and updates it when pipeline succeeds.

```rust
pub struct EvolutionCoordinator {
    // ... existing fields
    genome_config: Arc<Mutex<GenomeConfig>>,
}

impl EvolutionCoordinator {
    pub fn new(config: EvolutionConfig) -> Result<Self> {
        // ...
        Ok(Self {
            // ...
            genome_config: Arc::new(Mutex::new(GenomeConfig::default())),
        })
    }

    pub fn with_genome_config(mut self, config: GenomeConfig) -> Self {
        self.genome_config = Arc::new(Mutex::new(config));
        self
    }

    pub async fn genome_config(&self) -> GenomeConfig {
        self.genome_config.lock().await.clone()
    }

    // In run_evolution(), after successful migration:
    // Update care weights based on what changed
}
```

### Step 2: Update AletheonRuntime.post_evolution to pull updated config

```rust
pub async fn post_evolution<M: MetaRuntimeOps>(
    &mut self,  // Note: &mut self now
    // ... existing params
    meta: &MorphogenesisPipeline<M>,
) -> Result<Option<EvolutionSummary>> {
    match &self.evolution {
        Some(coord) => {
            let summary = coord.post_turn(/* ... */).await?;

            // Pull updated genome config if evolution triggered
            if summary.evolution_triggered {
                let new_config = coord.genome_config().await;
                self.genome_config = new_config;
            }

            Ok(Some(summary))
        }
        None => Ok(None),
    }
}
```

---

## Task 4: Reasoner 支持 care-aware reasoning

**Files:** `crates/aletheon-brain/src/core/reasoner.rs`

### Step 1: Add care-aware reasoning method

```rust
impl Reasoner {
    /// Think with care awareness — includes agent's values in the reasoning chain.
    pub fn think_with_care(
        &self,
        intent: &Intent,
        ctx: &Context,
        world_state: &str,
        care_weights: &std::collections::HashMap<String, f64>,
    ) -> String {
        let base = self.think(intent, ctx, world_state);
        if care_weights.is_empty() {
            return base;
        }

        let care_section = {
            let mut parts: Vec<String> = care_weights.iter()
                .map(|(k, v)| format!("  {}: {:.2}", k, v))
                .collect();
            parts.sort();
            format!("\nCare priorities:\n{}", parts.join("\n"))
        };

        // Inject care into the risk assessment step for CoT, or append for Direct
        if base.contains("Step 4") {
            base.replace(
                "Step 4 — Risk:",
                &format!("Step 4 — Risk (values-aware):{}\nConsider how actions align with care priorities above.", care_section)
            )
        } else {
            format!("{}{}", base, care_section)
        }
    }
}
```

### Step 2: Add tests

```rust
#[test]
fn test_care_aware_direct() {
    let reasoner = Reasoner::new(ReasoningStrategy::Direct);
    let mut care = std::collections::HashMap::new();
    care.insert("safety".to_string(), 1.0);
    let result = reasoner.think_with_care(&make_intent(), &make_ctx(), "", &care);
    assert!(result.contains("safety: 1.00"));
    assert!(result.contains("Care priorities"));
}

#[test]
fn test_care_aware_cot() {
    let reasoner = Reasoner::new(ReasoningStrategy::ChainOfThought);
    let mut care = std::collections::HashMap::new();
    care.insert("safety".to_string(), 1.0);
    let result = reasoner.think_with_care(&make_intent(), &make_ctx(), "", &care);
    assert!(result.contains("values-aware"));
}

#[test]
fn test_care_aware_empty_weights() {
    let reasoner = Reasoner::new(ReasoningStrategy::Direct);
    let care = std::collections::HashMap::new();
    let result = reasoner.think_with_care(&make_intent(), &make_ctx(), "", &care);
    assert!(!result.contains("Care priorities"));
}
```

---

## Task 5: 集成测试

**Files:** `crates/aletheon-runtime/tests/genome_runtime_mapping.rs`

```rust
use aletheon_runtime::core::config::GenomeConfig;
use aletheon_runtime::core::orchestrator::AletheonRuntime;
use aletheon_runtime::core::config::RuntimeConfig;

#[test]
fn test_genome_config_default() {
    let config = GenomeConfig::default();
    assert_eq!(config.reasoning_strategy, "plan-then-execute");
    assert_eq!(config.impasse_threshold, 0.3);
}

#[test]
fn test_runtime_holds_genome_config() {
    let mut runtime = AletheonRuntime::new(RuntimeConfig::default());
    let mut gc = GenomeConfig::default();
    gc.care_weights.insert("safety".to_string(), 1.0);
    runtime.update_genome_config(gc);
    assert_eq!(runtime.genome_config().care_weights.get("safety"), Some(&1.0));
}

#[test]
fn test_genome_config_update_changes_behavior() {
    let mut runtime = AletheonRuntime::new(RuntimeConfig::default());

    // Initial: plan-then-execute
    assert_eq!(runtime.genome_config().reasoning_strategy, "plan-then-execute");

    // Mutate
    let mut gc = GenomeConfig::default();
    gc.reasoning_strategy = "direct".to_string();
    runtime.update_genome_config(gc);

    assert_eq!(runtime.genome_config().reasoning_strategy, "direct");
}

#[test]
fn test_care_weights_in_prompt() {
    let mut gc = GenomeConfig::default();
    gc.care_weights.insert("safety".to_string(), 0.9);
    gc.care_weights.insert("helpfulness".to_string(), 0.7);
    let prompt = gc.care_weights_prompt();
    assert!(prompt.contains("safety: 0.90"));
    assert!(prompt.contains("helpfulness: 0.70"));
}
```

---

## Task 6: 提交和验证

```bash
cargo test -p aletheon-runtime -- genome_runtime_mapping
cargo test --workspace
git add -A
git commit -m "feat(runtime): wire GenomeConfig into AletheonRuntime

Genome reasoning_config and care_ext now influence runtime behavior:
- GenomeConfig holds strategy, impasse threshold, care weights
- AletheonRuntime reads and updates genome config
- Reasoner gains think_with_care() for values-aware reasoning
- EvolutionCoordinator updates config after successful migration
- Care weights injected into system prompt

Co-Authored-By: Claude <noreply@anthropic.com>"
```
