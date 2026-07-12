//! Code graph tool — AST-based code structure analysis using tree-sitter.

use super::{PermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta};
use async_trait::async_trait;
use serde::Serialize;
use serde_json::{json, Value};

pub struct CodeGraphTool;

/// A single symbol occurrence found by the code graph.
#[derive(Debug, Serialize)]
struct SymbolHit {
    name: String,
    kind: String,
    file: String,
    line: usize,
    column: usize,
}

#[async_trait]
impl Tool for CodeGraphTool {
    fn name(&self) -> &str {
        "code_graph"
    }

    fn description(&self) -> &str {
        "Query code structure using AST analysis — find symbols, callers, references"
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["symbols", "callers", "refs"]
                },
                "file_path": {
                    "type": "string"
                },
                "symbol": {
                    "type": "string"
                }
            },
            "required": ["operation"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L0
    }

    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(CodeGraphTool)
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> ToolResult {
        let start = ctx.clock.mono_now();

        let result = run_operation(&input).await;

        match result {
            Ok(hits) => {
                let content = serde_json::to_string_pretty(&hits).unwrap_or_else(|_| "[]".into());
                ToolResult {
                    content,
                    is_error: false,
                    metadata: ToolResultMeta {
                        execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                        truncated: false,
                    },
                }
            }
            Err(e) => ToolResult {
                content: format!("Error: {e}"),
                is_error: true,
                metadata: ToolResultMeta {
                    execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                    truncated: false,
                },
            },
        }
    }
}

/// Parse the operation and dispatch.
async fn run_operation(input: &Value) -> Result<Vec<SymbolHit>, String> {
    let op = input
        .get("operation")
        .and_then(|v| v.as_str())
        .ok_or("missing required field 'operation'")?;

    match op {
        "symbols" => {
            let path = input
                .get("file_path")
                .and_then(|v| v.as_str())
                .ok_or("'symbols' requires 'file_path'")?;
            extract_symbols(path)
        }
        "callers" => {
            let symbol = input
                .get("symbol")
                .and_then(|v| v.as_str())
                .ok_or("'callers' requires 'symbol'")?;
            let root = input
                .get("file_path")
                .and_then(|v| v.as_str())
                .unwrap_or(".");
            find_callers(symbol, root)
        }
        "refs" => {
            let symbol = input
                .get("symbol")
                .and_then(|v| v.as_str())
                .ok_or("'refs' requires 'symbol'")?;
            let root = input
                .get("file_path")
                .and_then(|v| v.as_str())
                .unwrap_or(".");
            find_refs(symbol, root)
        }
        other => Err(format!("unknown operation: {other}")),
    }
}

/// Create a tree-sitter Rust parser.
fn make_rust_parser() -> tree_sitter::Parser {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .expect("Error loading Rust grammar");
    parser
}

/// Walk a directory for `.rs` files, skipping common non-source dirs.
fn walk_rust_files(root: &str) -> Vec<std::path::PathBuf> {
    let skip = ["target", ".git", "node_modules", ".cargo"];
    walkdir::WalkDir::new(root)
        .into_iter()
        .filter_entry(|e| !skip.iter().any(|s| e.file_name().to_string_lossy() == *s))
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|ext| ext == "rs").unwrap_or(false))
        .map(|e| e.into_path())
        .collect()
}

/// Extract all function/struct/enum definitions from a single file.
fn extract_symbols(file_path: &str) -> Result<Vec<SymbolHit>, String> {
    let source =
        std::fs::read_to_string(file_path).map_err(|e| format!("read {file_path}: {e}"))?;

    let mut parser = make_rust_parser();
    let tree = parser
        .parse(&source, None)
        .ok_or_else(|| "tree-sitter parse failed".to_string())?;

    let mut hits = Vec::new();
    collect_definitions(&tree.root_node(), &source, file_path, &mut hits);
    Ok(hits)
}

/// Recursively collect definitions (function_item, struct_item, enum_item).
fn collect_definitions(
    node: &tree_sitter::Node,
    source: &str,
    file_path: &str,
    out: &mut Vec<SymbolHit>,
) {
    let kind_str = node.kind();
    if matches!(
        kind_str,
        "function_item" | "struct_item" | "enum_item" | "impl_item" | "trait_item"
    ) {
        let kind = match kind_str {
            "function_item" => "function",
            "struct_item" => "struct",
            "enum_item" => "enum",
            "impl_item" => "impl",
            "trait_item" => "trait",
            _ => "unknown",
        };
        // The name is typically the first "identifier" child.
        if let Some(name_node) = find_child_field(node, "name") {
            let name = name_node
                .utf8_text(source.as_bytes())
                .unwrap_or("<?>")
                .to_string();
            let pos = name_node.start_position();
            out.push(SymbolHit {
                name,
                kind: kind.to_string(),
                file: file_path.to_string(),
                line: pos.row + 1,
                column: pos.column + 1,
            });
        }
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_definitions(&child, source, file_path, out);
    }
}

/// Try to find the child with field name "name".
fn find_child_field<'a>(
    node: &tree_sitter::Node<'a>,
    field: &str,
) -> Option<tree_sitter::Node<'a>> {
    // Use child_by_field_name first (tree-sitter 0.24 API).
    if let Some(child) = node.child_by_field_name(field) {
        return Some(child);
    }
    // Fallback: for some node kinds the name is just the first identifier child.
    if field == "name" {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if child.kind() == "identifier" || child.kind() == "type_identifier" {
                return Some(child);
            }
        }
    }
    None
}

/// Find all call sites of a named function across the project.
fn find_callers(symbol: &str, root: &str) -> Result<Vec<SymbolHit>, String> {
    let files = walk_rust_files(root);
    let mut parser = make_rust_parser();
    let mut hits = Vec::new();

    for path in &files {
        let source = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let tree = match parser.parse(&source, None) {
            Some(t) => t,
            None => continue,
        };
        let path_str = path.display().to_string();
        collect_callers(&tree.root_node(), &source, symbol, &path_str, &mut hits);
    }
    Ok(hits)
}

/// Walk a subtree looking for call_expression nodes whose function name matches `symbol`.
fn collect_callers(
    node: &tree_sitter::Node,
    source: &str,
    symbol: &str,
    file_path: &str,
    out: &mut Vec<SymbolHit>,
) {
    if node.kind() == "call_expression" {
        if let Some(func_node) = node.child_by_field_name("function") {
            let text = func_node.utf8_text(source.as_bytes()).unwrap_or("");
            // Direct call: symbol(...) or path::symbol(...)
            let simple_name = text.rsplit("::").next().unwrap_or(text);
            if simple_name == symbol {
                let pos = func_node.start_position();
                out.push(SymbolHit {
                    name: symbol.to_string(),
                    kind: "call".to_string(),
                    file: file_path.to_string(),
                    line: pos.row + 1,
                    column: pos.column + 1,
                });
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_callers(&child, source, symbol, file_path, out);
    }
}

/// Find all textual references to a symbol (identifier usage, type references, etc.).
fn find_refs(symbol: &str, root: &str) -> Result<Vec<SymbolHit>, String> {
    let files = walk_rust_files(root);
    let mut parser = make_rust_parser();
    let mut hits = Vec::new();

    for path in &files {
        let source = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let tree = match parser.parse(&source, None) {
            Some(t) => t,
            None => continue,
        };
        let path_str = path.display().to_string();
        collect_refs(&tree.root_node(), &source, symbol, &path_str, &mut hits);
    }
    Ok(hits)
}

/// Walk a subtree looking for identifier nodes matching `symbol`.
fn collect_refs(
    node: &tree_sitter::Node,
    source: &str,
    symbol: &str,
    file_path: &str,
    out: &mut Vec<SymbolHit>,
) {
    let kind = node.kind();
    if kind == "identifier" || kind == "type_identifier" {
        if let Ok(text) = node.utf8_text(source.as_bytes()) {
            if text == symbol {
                let pos = node.start_position();
                out.push(SymbolHit {
                    name: symbol.to_string(),
                    kind: "reference".to_string(),
                    file: file_path.to_string(),
                    line: pos.row + 1,
                    column: pos.column + 1,
                });
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_refs(&child, source, symbol, file_path, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Helper: write a Rust snippet to a temp file, return its path.
    fn write_tmp_rust(name: &str, content: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::Builder::new()
            .suffix(".rs")
            .prefix(name)
            .tempfile()
            .unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f.flush().unwrap();
        f
    }

    #[test]
    fn test_extract_symbols() {
        let f = write_tmp_rust(
            "sym_",
            r#"
fn main() {}
struct Foo { x: i32 }
enum Bar { A, B }
trait Baz { fn qux(&self); }
"#,
        );
        let hits = extract_symbols(f.path().to_str().unwrap()).unwrap();
        let names: Vec<&str> = hits.iter().map(|h| h.name.as_str()).collect();
        assert!(names.contains(&"main"), "expected main, got {names:?}");
        assert!(names.contains(&"Foo"), "expected Foo, got {names:?}");
        assert!(names.contains(&"Bar"), "expected Bar, got {names:?}");
        assert!(names.contains(&"Baz"), "expected Baz, got {names:?}");
    }

    #[test]
    fn test_extract_symbols_kinds() {
        let f = write_tmp_rust(
            "kind_",
            r#"
fn hello() {}
struct World;
"#,
        );
        let hits = extract_symbols(f.path().to_str().unwrap()).unwrap();
        assert_eq!(hits[0].kind, "function");
        assert_eq!(hits[1].kind, "struct");
    }

    #[test]
    fn test_find_callers_in_project() {
        let dir = tempfile::tempdir().unwrap();
        let file_a = dir.path().join("a.rs");
        let file_b = dir.path().join("b.rs");
        std::fs::write(
            &file_a,
            r#"
fn do_thing() { helper(); }
fn helper() {}
"#,
        )
        .unwrap();
        std::fs::write(
            &file_b,
            r#"
fn other() { helper(); }
"#,
        )
        .unwrap();

        let hits = find_callers("helper", dir.path().to_str().unwrap()).unwrap();
        assert_eq!(hits.len(), 2, "expected 2 callers, got {hits:?}");
        assert!(hits[0].kind == "call");
    }

    #[test]
    fn test_find_refs_in_project() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("c.rs");
        std::fs::write(
            &file,
            r#"
struct Config { value: i32 }
fn make() -> Config { Config { value: 42 } }
"#,
        )
        .unwrap();

        let hits = find_refs("Config", dir.path().to_str().unwrap()).unwrap();
        assert!(hits.len() >= 3, "expected >=3 refs to Config, got {hits:?}");
    }

    #[test]
    fn test_symbols_nonexistent_file() {
        let result = extract_symbols("/nonexistent/path.rs");
        assert!(result.is_err());
    }

    #[test]
    fn test_unknown_operation() {
        let input = json!({"operation": "bogus"});
        let result = futures::executor::block_on(run_operation(&input));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unknown operation"));
    }
}
