# Plugin Subsystem

> New document — code paths updated to match actual crate names (fabric, cognit, corpus, dasein, mnemosyne, metacog, interact, executive)

> Plugin system for extending Aletheon with external tools and hooks, supporting command-based plugins with manifest-driven discovery and dependency resolution.

**Crate:** `executive`
**Code location:** `executive/src/impl/plugin/`
**Last Updated:** 2026-06-14

---

## Implementation Status

| Component | Status | Code Location | Notes |
|-----------|--------|---------------|-------|
| PluginManifest | Implemented | `executive/src/impl/plugin/manifest.rs` | TOML-based plugin manifest with validation |
| PluginLoader | Implemented | `executive/src/impl/plugin/loader.rs` | Discovery and dependency resolution |
| PluginManager | Implemented | `executive/src/impl/plugin/manager.rs` | Lifecycle management, tool creation |
| PluginRuntime | Implemented | `executive/src/impl/plugin/runtime.rs` | Command-based execution (native/WASM/agent planned) |

---

## 1. Overview

The plugin subsystem enables extending Aletheon with external tools and hooks. Plugins are discovered from configured directories, loaded via manifest files (`plugin.toml`), and integrated into the tool registry. Each plugin can provide tools and hooks.

---

## 2. Plugin Manifest (PluginManifest)

Plugins are defined by `plugin.toml` files. The manifest supports two formats:
- **Flat (legacy):** fields at top level
- **Nested (new):** `[plugin]` section with `entry` field using type prefix

```toml
id = "my-plugin"
name = "My Plugin"
version = "0.1.0"
description = "A plugin that does X"
author = "developer"
entry = "cmd:./run.sh"

[[tools]]
name = "search"
description = "Search tool"
input_schema = {}
permission_level = "L0"

[[hooks]]
event = "tool_call_completed"
handler = "./hooks/on_tool.sh"

[[dependencies]]
id = "other-plugin"
version_req = ">=0.1.0"
optional = false

[plugin_permissions]
filesystem = ["/tmp/*"]
network = ["*.example.com"]
```

**Key fields:**
- `entry` — Entry point with type prefix: `cmd:<path>` (subprocess), `native:<path>` (shared library, planned), `wasm:<path>` (WASM, planned), `agent:<id>` (agent plugin, planned)
- `tools` — Array of tool definitions (name, description, input_schema, permission_level)
- `hooks` — Array of hook definitions (event, handler)
- `dependencies` — Plugin dependencies with optional flag
- `plugin_permissions` — Structured permissions (filesystem, network)

Code location: `executive/src/impl/plugin/manifest.rs`

---

## 3. Plugin Loader (PluginLoader)

Discovers plugins from configured search directories and resolves dependency order.

**Discovery flow:**
1. Scan search directories for subdirectories containing `plugin.toml`
2. Parse and validate each manifest
3. Resolve dependencies into topologically sorted load order

**Dependency resolution:**
- Checks all non-optional dependencies are available
- Returns error if required dependency is missing
- Topological sort ensures dependencies load before dependents

Code location: `executive/src/impl/plugin/loader.rs`

---

## 4. Plugin Manager (PluginManager)

Manages plugin lifecycle: discovery, loading, tool creation, and unloading.

**Plugin lifecycle states:**
- `Discovered` — Manifest found but not loaded
- `Loaded` — Successfully loaded, tools available
- `Active` — Currently in use
- `Error(String)` — Failed to load (with error message)
- `Unloaded` — Explicitly unloaded

**Key operations:**
- `load_all()` — Discover, resolve dependencies, load all plugins
- `get_tools()` — Get all active plugin tools (implements `base::tool::Tool` trait)
- `unload(plugin_id)` — Unload a specific plugin

**Tool creation:** Each tool defined in a plugin manifest is wrapped in a `PluginTool` struct that implements the `Tool` trait. Tool execution delegates to the plugin's runtime.

Code location: `executive/src/impl/plugin/manager.rs`

---

## 5. Plugin Runtime (PluginRuntime)

Defines how plugin tools are executed.

**Supported runtime types:**
- `Command` — Run as subprocess (implemented): `cmd:<path>` prefix, executes with `--tool <name> --args <json>` arguments
- `Native` — Shared library loading (planned): `native:<path>` prefix
- `WASM` — WebAssembly execution (planned): `wasm:<path>` prefix
- `Agent` — Agent-based plugin (planned): `agent:<id>` prefix

**Command runtime execution flow:**
1. Resolve full path from plugin directory + entry path
2. Execute subprocess with `--tool <tool_name> --args <json_args>`
3. Parse stdout as JSON result
4. Return error with stderr on failure

Code location: `executive/src/impl/plugin/runtime.rs`

---

## 6. Design Notes

- **Permission model:** Each tool declares a permission level (L0-L3) in the manifest, parsed and enforced via the standard `Tool::permission_level()` interface
- **Error handling:** Plugin load failures are logged and the plugin enters `Error` state; other plugins continue loading
- **Thread safety:** `PluginManager` uses `RwLock<HashMap<String, ManagedPlugin>>` for concurrent access
- **Future extensions:** Native (.so), WASM, and Agent runtime types are stubbed but not yet implemented

---

## Implementation Summary

| Component | Code Location | Key Types |
|-----------|---------------|-----------|
| PluginManifest | `executive/src/impl/plugin/manifest.rs` | `PluginManifest`, `PluginToolDef`, `PluginHookDef`, `PluginDependency`, `PluginPermissions` |
| PluginLoader | `executive/src/impl/plugin/loader.rs` | `PluginLoader` |
| PluginManager | `executive/src/impl/plugin/manager.rs` | `PluginManager`, `PluginState`, `ManagedPlugin`, `PluginTool` |
| PluginRuntime | `executive/src/impl/plugin/runtime.rs` | `PluginRuntime` (Command variant) |

**Test coverage:** manifest.rs has 5 tests (validation, TOML parsing, entry type parsing). manager.rs has integration tests for tool creation and execution. automation/mod.rs has 10+ tests for scheduler lifecycle.
