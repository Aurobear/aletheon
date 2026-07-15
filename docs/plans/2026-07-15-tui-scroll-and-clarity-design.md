# TUI Scroll Performance and Clarity Design

## Problem

The chat viewport becomes visibly sluggish while scrolling because every frame
re-renders and word-wraps the complete transcript before copying only the
visible tail. The same screen is visually noisy: response paragraphs inherit a
repeated left rail, Markdown tables use dense borders, routine tool output is
shown alongside the answer, and infrastructure diagnostics can leak into the
final response as user instructions.

## Desired behavior

- Scrolling remains responsive as transcript length grows.
- Static screens do not redraw on a fixed timer.
- Streaming and spinner animation remain smooth without forcing expensive
  transcript layout on every frame.
- Answers resemble Codex's text-first hierarchy: plain prose, restrained
  headings, concise semantic tool activities, and errors only when actionable.
- A sandbox-specific Git ownership workaround is scoped to the validated
  working directory and is never presented as a global configuration command.

## Design

### Cached transcript layout

`ChatWidget` owns a cached, wrapped transcript keyed by content revision and
viewport width. Adding or updating an entry, expanding a tool result, or
resizing the terminal invalidates the cache. Ordinary scroll operations reuse
the cache and select only the visible slice. Rendering must not clone or wrap
the complete transcript during every frame.

Animated tool activity is intentionally excluded from the transcript cache's
content-dependent text. Its small visible indicator may be overlaid or updated
without rebuilding completed history.

### Event-driven redraw

The lifecycle loop redraws when input, socket data, resize, scrolling, or UI
state changes. A timer is retained only while an animation or pending IME
submission requires it. Blocking terminal event polling must not prevent the
Tokio socket from being serviced.

### Text-first presentation

- Assistant prose has no repeated vertical rail.
- Markdown tables render as lightweight aligned rows rather than boxed grids.
- Headings use typography and spacing rather than emoji decoration.
- Completed tools use one semantic line such as `• Read Cargo.toml`.
- Successful tool stdout stays collapsed; failures show a short excerpt.
- Routine `Spec: on track. Continuing...` reflections are not displayed.

### Git sandbox behavior

The command runner injects `safe.directory` through per-process Git
environment variables for the validated current working directory. It does not
modify global Git configuration and never uses `safe.directory=*`.

## Validation

- Unit tests cover cache invalidation, cache reuse during scrolling, semantic
  tool activity, failure excerpts, and suppressed routine reflection.
- A long synthetic transcript is scrolled repeatedly while asserting layout is
  not rebuilt.
- The real deployed TUI is exercised from
  `/home/aurobear/Bear-ws/aletheon` and checked for responsive scrolling,
  returned prompt, substantive final answer, concise activities, and absence of
  output-size hints, routine reflection, false Git errors, and global Git
  configuration advice.

## Scope

This change is limited to the terminal lifecycle, chat/Markdown presentation,
and the already-required scoped Git runner environment. It does not replace
Ratatui, redesign the daemon protocol, or add persistent UI preferences.
