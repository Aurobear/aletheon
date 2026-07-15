# Client Working Directory and Tester Reliability Design

## Problem

Interactive clients send only chat text. The systemd daemon therefore builds each
turn with its own process directory (`/`) and cannot identify the project from
which the user launched Aletheon. Models compensate by scanning the host and can
misidentify an unrelated directory as the current project. The tester also
assumes its monitor MCP tools are installed and does not prove that source,
binary, service configuration, and real TUI behavior refer to the same build.

## Runtime design

Every local chat client sends a canonical `working_dir` beside `message`. The
daemon accepts only an existing canonical directory below
`/home/aurobear/Bear-ws`, rejects invalid or out-of-policy paths, and carries the
validated path through `TurnRequest` into all tool contexts. Requests that omit
the field retain the daemon data directory for compatibility with non-local
channels; they must not trigger a host-wide project search.

The systemd service exposes `/home/aurobear/Bear-ws` to the daemon. Bubblewrap
binds only the validated working directory as writable for a turn. The TUI gains
`/cwd` so the effective client directory is observable. Session events record
the working directory for diagnosis without placing it inside user text.

## Sandbox correction

The bubblewrap launcher must make `/dev/null` writable in the real systemd user
namespace. Its regression test executes redirection through the deployed service
environment rather than checking argument presence alone.

## Tester design

`aletheon-tester` performs capability detection first. When monitor MCP tools are
unavailable it uses the shipped monitor Python package when possible, otherwise
falls back to CLI, tmux, session JSONL, and journalctl. Preflight records the
active worktree, commit, binary metadata, unit properties, daemon start time, and
client cwd.

Tests use explicit assertions: required successful tools, forbidden errors,
visible final answer, expected cwd, and forbidden host-wide scanning. TUI
completion is detected from a returned prompt plus stable frame hashes rather
than a fixed sleep. Nondeterministic model paths run three times. Each report
contains commands, frames, session identifier, tool inputs/results, and a final
pass/fail verdict.

## Acceptance criteria

- Starting `aletheon` in a project and asking for the current project operates
  on that canonical directory.
- A client path outside `/home/aurobear/Bear-ws` is rejected explicitly.
- The daemon never substitutes `/` or scans unrelated host directories to infer
  a missing project.
- `bash_exec` supports `/dev/null` redirection in the deployed systemd service.
- The tester works with or without monitor MCP registration and identifies the
  exact running build.
- Three real TUI runs pass cwd, sandbox, final-answer, and rendering assertions.
