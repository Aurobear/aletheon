use fabric::tool::ConcurrencyClass;

/// Maximum number of tools to execute in a single parallel batch.
const MAX_PARALLEL_TOOLS: usize = 8;

/// A batch of tool calls classified for parallel or serial execution.
pub enum ToolBatch {
    /// Read-only tools safe to execute concurrently.
    Parallel(Vec<(String, String, serde_json::Value)>),
    /// Side-effect tools that must be executed sequentially.
    Serial(Vec<(String, String, serde_json::Value)>),
}

/// Classify a tool by name into its concurrency class.
fn classify_tool(tool_name: &str) -> ConcurrencyClass {
    match tool_name {
        "read_file" | "glob" | "grep" | "file_read" | "system_status" | "process_list"
        | "memory_search" | "ls" | "web_fetch" | "web_search" => ConcurrencyClass::ReadOnly,
        _ => ConcurrencyClass::SideEffect,
    }
}

/// Partition a list of tool calls into parallel and serial batches.
///
/// Contiguous read-only tools are grouped into a single `Parallel` batch
/// (up to `MAX_PARALLEL_TOOLS`). Each side-effect tool gets its own `Serial`
/// batch. A switch from read-only to side-effect (or vice versa) flushes the
/// current batch.
pub fn partition_tool_calls(calls: &[(String, String, serde_json::Value)]) -> Vec<ToolBatch> {
    if calls.is_empty() {
        return Vec::new();
    }

    let mut batches: Vec<ToolBatch> = Vec::new();
    let mut current_readonly: Vec<(String, String, serde_json::Value)> = Vec::new();
    let mut current_serial: Vec<(String, String, serde_json::Value)> = Vec::new();

    let flush_readonly =
        |batches: &mut Vec<ToolBatch>, buf: &mut Vec<(String, String, serde_json::Value)>| {
            if !buf.is_empty() {
                batches.push(ToolBatch::Parallel(std::mem::take(buf)));
            }
        };

    let flush_serial = |batches: &mut Vec<ToolBatch>,
                        buf: &mut Vec<(String, String, serde_json::Value)>| {
        if !buf.is_empty() {
            batches.push(ToolBatch::Serial(std::mem::take(buf)));
        }
    };

    for call in calls {
        let class = classify_tool(&call.1);
        match class {
            ConcurrencyClass::ReadOnly => {
                flush_serial(&mut batches, &mut current_serial);
                current_readonly.push(call.clone());
                if current_readonly.len() >= MAX_PARALLEL_TOOLS {
                    flush_readonly(&mut batches, &mut current_readonly);
                }
            }
            _ => {
                flush_readonly(&mut batches, &mut current_readonly);
                current_serial.push(call.clone());
                // Each side-effect tool gets its own serial batch.
                flush_serial(&mut batches, &mut current_serial);
            }
        }
    }

    flush_readonly(&mut batches, &mut current_readonly);
    flush_serial(&mut batches, &mut current_serial);

    batches
}
