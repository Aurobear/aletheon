# RA-05 Runtime Agent admission/projection production cutover — 2026-08-13

The daemon composition root now constructs `runtime::AgentAdmissionPolicy` and
`runtime::BoundedAgentAdmission` directly from validated host configuration,
rather than instantiating the Executive compatibility shell. Runtime therefore
owns the actual bounded admission object used in production.

The one-way Agent lifecycle/SQL projection bridge now lives at
`runtime::RuntimeAgentRunProjection`, and Aletheon composes that owner directly.
The existing Executive copy remains temporarily for Executive-only tests until
the corresponding test migration/deletion slice.

Validation:

- `bash scripts/cargo-agent.sh check -p runtime --all-targets`: passed.
- `bash scripts/cargo-agent.sh test -p runtime --lib`: 137 passed.
- `bash scripts/cargo-agent.sh check -p aletheon --all-targets`: passed.
