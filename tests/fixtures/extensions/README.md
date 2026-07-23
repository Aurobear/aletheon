# Extension Platform Fixtures

These fixtures are for extension platform baseline testing.

## Packages

- **legal-minimal**: A minimal valid extension package for baseline testing.
  Contains a single `skill` asset (`skill.demo`) with a valid SKILL.md.

- **malicious-path-escape**: A package attempting a path traversal attack.
  The `path` field in `extension.toml` contains `../` sequences designed to
  escape the expected asset directory.

## Usage

These fixtures are used by Phase 2 Package Inspector tests. The inspector
validates `extension.toml` structure and rejects packages whose asset paths
contain path-traversal sequences.

## Daemon Crash-Loop Repro

When an Agent Profile references an unknown tool, the daemon currently exits
with error instead of surfacing a usable diagnostic. This causes a crash-loop
in the embodiment bootstrap path. See Phase 3 for the planned fix.
