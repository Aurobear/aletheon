# Architecture gate fixtures

`tests/architecture_check.sh` builds temporary repositories and injects one
violation at a time. The fixtures prove that the production gate rejects:

- unregistered workspace crates and top-level `impl/` directories;
- external product identifiers in protected core paths;
- application-to-adapter imports;
- public adapter/implementation-tree exports;
- provider-name business branches;
- unregistered wire and persistence surfaces;
- compatibility debt above its ratcheted baseline;
- provider-specific JSON field inspection in core code.

The same fixtures also prove that composition-only adapter factory matching and
opaque adapter JSON payloads remain legal. No fixture is compiled as production
code.
