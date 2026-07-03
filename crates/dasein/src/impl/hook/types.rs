use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HookEventName {
    PreToolUse,
    PostToolUse,
    PermissionRequest,
    PreLLMCall,
    PostLLMCall,
    SessionStart,
    UserPromptSubmit,
    SubagentStart,
    SubagentStop,
    Stop,
    PreCompact,
    PostCompact,
    PerceptionEvent,
    SecurityViolation,
    /// Before building a kernel module or kernel
    PreKernelBuild,
    /// After building a kernel module or kernel
    PostKernelBuild,
    /// Before loading a kernel module
    PreModuleLoad,
    /// After loading a kernel module
    PostModuleLoad,
    /// Before compiling eBPF program
    PreEbpfCompile,
    /// After compiling eBPF program
    PostEbpfCompile,
    /// Bottleneck detected
    BottleneckDetected,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HookType {
    /// Run subprocess
    Command,
    /// Inject text into context
    Prompt,
    /// Delegate to sub-agent
    Agent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionMode {
    Sync,
    Async,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HookScope {
    Thread,
    Turn,
    Global,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HookSource {
    System,
    User,
    Project,
    Plugin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HookTrustStatus {
    Managed,
    Trusted,
    Modified,
    Untrusted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookMatcher {
    pub tool: Option<String>,
    pub args_pattern: Option<String>,
    pub risk_category: Option<String>,
}

impl HookMatcher {
    pub fn matches(&self, tool: Option<&str>, args: Option<&str>, risk: Option<&str>) -> bool {
        if let Some(ref t) = self.tool {
            if tool.is_none_or(|t2| t2 != *t) {
                return false;
            }
        }
        if let Some(ref pattern) = self.args_pattern {
            if args.is_none_or(|a| !a.contains(pattern.as_str())) {
                return false;
            }
        }
        if let Some(ref r) = self.risk_category {
            if risk.is_none_or(|r2| r2 != *r) {
                return false;
            }
        }
        true
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hook {
    pub id: String,
    pub name: String,
    pub event: HookEventName,
    pub matcher: HookMatcher,
    pub hook_type: HookType,
    pub execution: ExecutionMode,
    pub scope: HookScope,
    pub trust: HookTrustStatus,
    pub timeout_sec: u64,
    pub source: HookSource,
    pub enabled: bool,
    // For Command type
    pub command: Option<String>,
    pub command_args: Option<Vec<String>>,
    // For Prompt type
    pub prompt_text: Option<String>,
}

/// Context passed to hooks during execution.
pub struct HookContext {
    pub tool: Option<String>,
    pub args: Option<String>,
    pub risk: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug)]
pub enum HandlerResult {
    Continue,
    Block(String),
    ModifyArgs(Value),
    InjectContext(String),
    Failed(String),
    TimedOut,
}
