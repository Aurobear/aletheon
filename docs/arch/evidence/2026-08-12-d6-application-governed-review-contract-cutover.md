# D6 Application governed-review contract cutover evidence

- Date: 2026-08-12
- Scope: move bounded governed-review job/receipt contracts from the shared Fabric root to their declared Application review/approval owner.
- Canonical owner: `crates/application/src/governed_review.rs`.
- Contract tests moved to `crates/application/tests/governed_review_contract.rs`.
- Aletheon and Executive callers consume the Application contract directly.
- Compatibility: the Fabric module and re-export were deleted; nine census/public-baseline rows were retired and the baseline changed from 778 to 769.
