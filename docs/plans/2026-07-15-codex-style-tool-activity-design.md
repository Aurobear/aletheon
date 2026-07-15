# Codex-Style Tool Activity Design

## Goal

Keep Aletheon's normal conversation focused on the answer while retaining tool
progress and failures as useful diagnostics.

## Design

Successful tool calls render as one concise semantic action such as `• Read
Cargo.toml`, `• Searched crates`, or `• Ran git status`. Successful stdout and
size/expansion hints remain hidden by default. Failed calls show a clear failure
marker and the first error lines. Internal on-track reflection messages are not
shown; reflection remains visible only when it changes strategy or stops work.

Git commands receive a per-turn `safe.directory` value equal to the validated
working directory through Git's process environment. This fixes ownership
checks without trusting every repository on the host.

## Acceptance criteria

- Normal exploration does not display output character counts or `Ctrl+B` hints.
- Tool labels describe the action instead of exposing JSON function syntax.
- Git status and log work in the validated project directory.
- On-track reflection messages are absent from normal chat.
- Tool failures remain visible and expandable.
