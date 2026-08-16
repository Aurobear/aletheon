# Extension Package Runtime

The retired Executive plugin adapter is not a current subsystem. Extension
packages are composed and supervised by the Aletheon host, with package assets
resolved through Corpus:

- `crates/aletheon/src/extensions/mod.rs`
- `crates/aletheon/src/wiring/daemon/bootstrap/extensions.rs`
- `crates/aletheon/src/wiring/daemon/bootstrap/extension_provider_launcher.rs`
- `crates/corpus/src/extension/mod.rs`

Extension manifests, skills, hooks, connectors, executables, lifecycle state,
and subprocess providers must pass the package/runtime policy gates. Native
shared-library and WASM plugin claims from the former design are not current
production capabilities.
