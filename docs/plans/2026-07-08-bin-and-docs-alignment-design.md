# Binary Entry And Documentation Alignment

## Scope

1. Keep the 14 documentation deletions already present in the working tree.
2. Rename the unified executable package directory from `crates/aletheon` to
   `crates/bin` without changing the installed executable name `aletheon`.
3. Rename concept-oriented design directories to match their implementation
   crates:
   - `abi` -> `base`
   - `body` -> `corpus`
   - `brain` -> `cognit`
   - `self` -> `dasein`
   - `meta` -> `metacog`
   - `cli` -> `interact`
4. Update README and design documentation links and descriptions to reflect the
   current workspace, unified executable, service files, CI, and crate layout.

## Package Layout

```text
crates/bin/                 executable assembly only
    Cargo.toml              package: aletheon-bin
    src/main.rs             binary: aletheon

crates/base/                shared contracts and communication
crates/cognit/              cognition
crates/corpus/              tools, sandbox and perception
crates/dasein/              self model
crates/interact/            CLI/TUI implementation
crates/memory/              memory
crates/metacog/             meta-cognition and evolution
crates/runtime/             runtime and daemon implementation
```

`crates/bin` may depend on domain crates to assemble the application. Domain
crates must not depend on `aletheon-bin`.

## Documentation Rules

- Documentation paths use actual crate names rather than conceptual aliases.
- Concept names such as Brain, Body, Self, and MetaRuntime remain explanatory
  terms inside documents, not directory names.
- Links to deleted guide, architecture, development, and old plan documents are
  removed or redirected to existing documents.
- Capability statements distinguish implemented, partial, experimental, and
  design-only behavior.
- Volatile counts such as a fixed unit-test total are avoided unless generated.

## Validation

- `cargo metadata --no-deps` recognizes `crates/bin` and no longer references
  `crates/aletheon`.
- The workspace still produces a binary named `aletheon`.
- All relative Markdown links in `README.md` and `docs/**/*.md` resolve.
- Repository searches find no obsolete `crates/aletheon`, `docs/design/abi`,
  `docs/design/body`, `docs/design/brain`, `docs/design/self`,
  `docs/design/meta`, or `docs/design/cli` paths.
- Existing user deletions remain deleted.

