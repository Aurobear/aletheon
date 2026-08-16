# APX daemon lifecycle owner cutover — 2026-08-13

The typed daemon startup state machine, install-mode decision, startup lock and
host backend ports, readiness receipts, time-bounded polling and typed errors now
live in `application::daemon_lifecycle`. Concrete filesystem locking, systemd,
process spawn and Unix socket probing remain in `aletheon::wiring::readiness`.
All production callers use the Application owner directly; Executive retains
only a compatibility re-export.

Validation:

- `bash scripts/cargo-agent.sh test -p application --lib`: 41 passed.
- `bash scripts/cargo-agent.sh check -p executive --all-targets`: passed.
- `bash scripts/cargo-agent.sh check -p aletheon --all-targets`: passed.
