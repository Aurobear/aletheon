# Deferred Features

Status: **Not Started** | Priority: Future

---

## 1. TUI Redesign Phase 3-6 — Deep Streaming/Dedup

- **Source:** aletheon-tui-observability memory (2026-07-06)
- **Severity:** Feature
- **Description:** Phases 3-6 of the TUI redesign are deferred:
  - Phase 3: Deep streaming correctness (tool output interleaving)
  - Phase 4: Dedup/consolidation of streamed content
  - Phase 5: Claude-like streaming UX (live token rendering)
  - Phase 6: Multi-pane streaming (tools + thoughts in parallel)
- **Impact:** TUI streaming is basic; no live tool output; no parallel pane rendering.
- **Plan:** TBD — no detailed plan exists yet.

---

## 2. Phase 5 — Self-Evolution, Kernel Tools, Offline Models

- **Source:** aletheon-production-hardening memory (2026-07-06)
- **Severity:** Feature
- **Description:** Production hardening Phase 5 is gated — not started, spec not defined:
  - Self-evolution (`MorphogenesisPipeline` exists behind `--enable-evolution` flag)
  - Kernel-level tools (eBPF-based introspection and modification)
  - Offline model support (local LLM inference)
- **Impact:** Self-evolution is opt-in; no offline operation; no kernel-level tooling.
- **Plan:** Spec not yet written.

---

## 3. Clock Unification — Remaining 139 Direct Time Calls

- **Source:** aletheon-space-lifecycle memory (2026-07-13)
- **Severity:** Technical Debt
- **Description:** ~139 production code sites still use direct time calls instead of the `WallTime` abstraction:
  - mnemosyne: 33
  - executive: 32
  - dasein: 29
  - corpus: 19
  - fabric: 17
  - kernel: 8
  - cognit: 1
- **Impact:** Time is not injectable for testing; time-dependent behavior is non-deterministic.
- **Plan:** Per-crate migration batches; mnemosyne and executive first.
