# D6 owner-local resource and SpaceManager retirement

The Fabric ledger assigned `types/resource.rs` to owner-local implementation/delete and `include/space.rs` to an unresolved owner (`docs/plans/2026-08-09-fabric-source-disposition-ledger.md`). Current code evidence resolves both:

- `ManagedResource`/`ResourceState` had no caller outside their own file, so the unused generic lifecycle implementation was deleted.
- `SpaceManager` had one implementation and no trait-object consumer. `InMemorySpaceManager` is private to Kernel; the trait only caused Kernel to depend on a Fabric rich port. Its used `fork_space` operation is now an inherent Kernel method. The unused `attach_region` method and Fabric trait were deleted.
- No second space manager or resource lifecycle authority was introduced.
