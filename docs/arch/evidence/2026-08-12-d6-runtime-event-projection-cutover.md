# D6 Runtime event projection cutover evidence

- Date: 2026-08-12
- Canonical owner: `runtime::event_projection`.
- The bounded schema-filtered notification bus, subscription registry, subscription IDs now live together under Runtime.
- Aletheon, Cognit, Corpus, Dasein and Executive consume Runtime directly.
- Fabric implementation/re-exports, compatibility alias, tests, and nine Fabric census/public rows; unused legacy `EventType` and event `Priority` were deleted rather than retained in Runtime were removed; baseline changed from 751 to 742.
