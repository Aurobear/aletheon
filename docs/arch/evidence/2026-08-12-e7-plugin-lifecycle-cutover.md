# E7 extension plugin lifecycle cutover — 2026-08-12

## Requirement and code anchors

E7 owns deletion of extension-specific Fabric rich types and root re-exports
after their callers have switched to the extension owner.
The boundary
census assigned `Plugin` and `PluginContext` to the Corpus/Application extension
package and E7 deletion (`config/architecture/fabric-boundary-census.tsv:172-173`).
Before this slice, their only non-test consumer was the rollback Executive plugin
manager (`crates/executive/src/adapters/plugin/manager.rs:10`).

## Result

The lifecycle contract now lives with the extension package at
`crates/corpus/src/extension/plugin.rs:1-42` and is exported from
`crates/corpus/src/extension/mod.rs`. The Executive compatibility manager imports
that authority directly. Fabric's `include::plugin` module, root module export,
and root `Plugin`/`PluginContext` re-exports were deleted.

```text
Before: Executive compatibility host -> Fabric rich extension facade
After:  Executive compatibility host -> Corpus extension lifecycle contract
        Fabric plugin lifecycle row   -> deleted
```

Caller-zero assertions:

```bash
! rg -n 'fabric::(?:include::)?plugin|include::plugin' crates --glob '*.rs'
! test -e crates/fabric/src/include/plugin.rs
```

Focused validation:

```bash
bash scripts/cargo-agent.sh fmt --all -- --check
bash scripts/cargo-agent.sh test -p corpus extension::plugin --lib
bash scripts/cargo-agent.sh test -p executive adapters::plugin::manager::lifecycle_tests --lib
```
