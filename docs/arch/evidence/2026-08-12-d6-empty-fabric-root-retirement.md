# D6 empty Fabric root retirement evidence

- Date: 2026-08-12
- Deleted the empty `ipc::bus`, `kernel`, and `security` aggregation shells after their last owned implementations had already moved or been retired.
- Removed their parent module declarations.
- No compatibility module or public symbol was retained; non-empty `events`, `policy`, and `primitives` roots remain until their owned children move.
