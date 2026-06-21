# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Open-source readiness: CI/CD pipeline with GitHub Actions
- Open-source readiness: MIT license badges and quick start links
- Open-source readiness: Contributing guidelines
- Open-source readiness: Crate descriptions for all workspace members
- **IPC:** Added Transport implementations for io_uring and shared memory
- **IPC:** Added connection pooling to UnixSocketTransport
- **IPC:** Integrated JSON-RPC adapter into Transport system
- **Documentation:** Added README.md for 8 crates (base, cognit, dasein, corpus, memory, interact, drivers, tools)

### Changed
- Refactored runtime structure to core/bridge/impl/ pattern
- Refactored self-field to core/bridge/impl/ pattern
- Refactored brain-core to core/bridge/impl/ pattern
- Refactored body to core/bridge/impl/ pattern
- **Phase 5: Intra-crate modularization (Linux kernel style)**
  - `base`: 7 top-level modules (include/, types/, events/, ipc/, kernel/, policy/, dasein/)
  - `memory`: backends/ and ops/ organization
  - `cognit`: core/ split into 14 sub-modules
  - `interact`: ui/ split into 20+ sub-modules with app/ and render/ subdirs
  - `runtime`: handler, fact_store, react_loop split into sub-modules
- Updated all crate name references (aletheon-* to new names)

### Fixed
- **IPC:** Fixed SharedMemBackend ring buffer: corrected available() calculation with wrapping_sub, and fixed write()/read() to handle messages crossing buffer boundary
- **IPC:** Fixed IpcManager::as_transport() to use actual active backend
- **IPC:** Fixed IpcManager to store and use actual socket_dir instead of hardcoded /tmp

### Deprecated
- **IPC:** Marked KernelEventBus::request() as deprecated (use RequestResponseProtocol)

## [0.1.0] - 2026-06-06

### Added
- Initial release of Aletheon agent runtime
- Core architecture: Nous (Soul/Brain/Body) triune design
- Crate structure: 10 crates organized by domain
- ReAct cognitive engine with reasoning, planning, and reflection
- Memory system: episodic, semantic, and procedural memory
- Security model: L0-L3 permission tiers
- Linux platform integration: eBPF, FUSE, systemd
- Android platform design: AccessibilityService, Foreground Service
- Embedded support: RK3588, Jetson Orin Nano
- CLI and TUI interface
- MCP server definitions
- Agent definition system (TOML + Markdown)
