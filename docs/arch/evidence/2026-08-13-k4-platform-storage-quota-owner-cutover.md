# K4 Platform storage-quota owner cutover — 2026-08-13

Managed-root scanning, link-safety enforcement, limits and concurrent storage
reservations now live in `platform::storage_quota`. All Aletheon production
composition imports Platform directly. Executive retains a temporary re-export
for its artifact, goal and external-provider adapters.

Validation:

- `bash scripts/cargo-agent.sh test -p platform --lib`: 46 passed.
- `bash scripts/cargo-agent.sh check -p executive --all-targets`: passed.
- `bash scripts/cargo-agent.sh check -p aletheon --all-targets`: passed.
