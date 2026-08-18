# Legacy session compatibility sunset ledger

Status: **ACTIVE (compatibility) — tracked for sunset**

## Purpose

`crates/aletheon/src/daemon/legacy_session.rs` (`LegacySessionService`) backs the
legacy JSON-RPC `session.*` surface that predates the typed Gateway session
model. Per the wiring-ownership plan (M8), this compatibility path must either
be deleted or carried as an **explicit legacy adapter with a sunset ledger**.
It has live production consumers (the JSON-RPC `session.*` routes and the
CLI/TUI resume flow), so it is retained as a compatibility adapter while each
method is tracked below against its canonical typed-Gateway replacement.

Every entry names the legacy JSON-RPC method, its production consumers, the
canonical replacement, and the sunset disposition. A method is `SUNSET` only
when its consumers have been cut over to the typed Gateway path and the legacy
route is removed.

## Ledger

| Legacy session API | Backing impl | Consumers | Canonical typed-Gateway replacement | Disposition |
|---|---|---|---|---|
| `session.create` / `new_session` | `LegacySessionService::create` | `rpc_session.rs`, CLI | `Command::CreateSession` (typed.rs:496) | KEEP — mapped by Gateway |
| `session.new` (switch) | `create_and_switch` | `rpc_session.rs`, TUI | `Command::CreateSession` + `ForkSession` | KEEP |
| `session.resume` | `resume` | `rpc_session.rs`, CLI `resume` | `Command::ResumeSession` (typed.rs:507) | KEEP — mapped by Gateway |
| `session.list` | `list` / `list_available` | `rpc_session.rs` | `Command::SessionSnapshot` (session enumeration) | KEEP |
| `session.switch` | `switch` | `rpc_session.rs` | `Command::ResumeSession` + thread select | KEEP |
| `session.load_recent` | `load_recent` | `rpc_session.rs` | `SessionSnapshot` + recent cursor | KEEP |
| `session.load_previous` | `load_previous` | `rpc_session.rs` | `SessionSnapshot` + history cursor | KEEP |
| `session.clear` | `clear` | `rpc_session.rs` | typed thread reset / new session | KEEP |
| `session.compact` | `compact` | `rpc_session.rs` | canonical `ContextCompactionReceipt` via runtime | KEEP |
| `session.current` | `current` | internal | `SessionSnapshot` | KEEP |
| workspace route | `route_workspace` | `handler::select_workspace_session` | Gateway workspace session routing | KEEP |

## Sunset rule

A row becomes `SUNSET` only when **both** hold:

1. Its last production consumer has been cut over to a typed-Gateway command
   (no JSON-RPC `session.*` call site remains in `crates/aletheon/src`); and
2. The corresponding legacy method is removed from `LegacySessionService` and
   `rpc_session.rs` in the same change.

Until then the row stays `KEEP` — the compatibility adapter is the explicit
transition vehicle, not an untracked second authority.

## Migration target

The plan's end state moves the remaining JSON-RPC `session.*` translation into
the Gateway as an explicit legacy adapter (`crates/gateway/src/client/legacy.rs`
already carries the legacy client translation; `typed.rs:496-507` already maps
`new_session`/`session.create`/`resume`/`session.resume`). The
`LegacySessionService` implementation stays daemon-side until every ledger row
is `SUNSET`; moving the 575-LOC service into the Gateway crate is tracked as a
residual and will be revisited when the JSON-RPC session surface is retired.
