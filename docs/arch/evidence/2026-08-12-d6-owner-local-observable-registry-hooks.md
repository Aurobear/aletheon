# D6 owner-local observable, registry, and hook closeout

- The generic Fabric registry had one production implementation: Corpus's
  `ToolRegistry`. Its handle and narrow trait now live beside that owner, and
  Executive composition imports the Corpus-owned extension trait directly.
- Fabric `Observable`/`SubsystemStatus` had one implementation in Interact's
  ACP adapter and no cross-component consumer. The unused shared abstraction
  was retired; ACP retains its bounded inherent metrics API.
- The legacy `hook_ext` DTO family had no caller. Corpus already owns the live
  hook loader schema, so D6 deleted the unused Fabric duplicate.
- Seven Fabric public rows and all compatibility re-exports were removed.
