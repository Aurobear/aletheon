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

### Changed
- Refactored runtime structure to core/bridge/impl/ pattern
- Refactored self-field to core/bridge/impl/ pattern
- Refactored brain-core to core/bridge/impl/ pattern
- Refactored body to core/bridge/impl/ pattern

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
