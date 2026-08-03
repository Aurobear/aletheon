# Runtime correctness and acceptance

> Verified against the installed-runtime workflow on 2026-07-26.

## Purpose

Aletheon is a persistent runtime, so a plausible answer is not sufficient
evidence of correctness. Acceptance combines the authority that selected and
executed the turn with what the user actually saw.

```text
typed host authority
  + installed binary provenance
  + rendered TUI frame
  + session JSONL
  + daemon journal
  + durable terminal state
  = acceptance evidence
```

## Runtime facts

The following are host-owned facts, never model-owned claims:

- effective model specification and display name;
- maximum context capacity and active context occupancy;
- provider retries and provider-advised cooldown;
- session, process, operation, Agent, and runtime identity;
- capabilities, tool authority, and token/tool/time budgets;
- deployment binary digest and running executable digest.

`fabric::ModelRuntimeFacts` is the shared inference contract. Routed providers
override the compatibility default with the resolved model specification.
`TurnPipeline` serializes those facts into the current system context after
model selection. The model may report them, but cannot author or override them.

## Independent metrics

Never collapse these counters into one “request” or “context” number:

| Metric | Meaning |
|---|---|
| user turns | accepted user inputs |
| inference rounds | model calls initiated by the cognitive loop |
| provider retries | repeat attempts caused by transient provider failure |
| tool calls | governed capability invocations |
| subagent inference | child-runtime model rounds |
| cumulative tokens | provider accounting over time |
| active context | tokens currently replayed in one request |
| cache tokens | provider cache hit/write accounting |

Tool reduction is not an optimization when it removes evidence. Cumulative
token usage is not context-window pressure.

## Async and event correctness

- Spawn returns a durable handle, not a child result.
- A terminal snapshot/event/receipt is required before reporting success.
- Retry creates or records an attempt; it does not erase failed history.
- Event schemas are semantic contracts. Runtime progress must not reuse a
  public Session/Turn schema.
- Projection poison is a failed acceptance even when unrelated reducers keep
  running.

## Required test lanes

### Deterministic lane

Use focused Rust contract tests through `scripts/cargo-agent.sh`. Cover typed
facts, protocol round trips, retry parsing, terminal recovery, schema/projector
compatibility, and tool lifecycle contracts.

### Fresh-session real-TUI lane

Run model-controlled routing/argument scenarios three consecutive times using
`/usr/bin/aletheon`. Each run must render a substantive answer, return `❯`, and
contain no forbidden provider/infrastructure error in its frame or journal.

### Sustained-session lane

Keep one TUI session open across multiple turns. Include a grounded task, a
follow-up, an unrelated short prompt, and—when runtime facts changed—identity
challenges. This detects poisoned history, stuck busy state, incorrect resume,
and reversion to model training priors.

### Installed provenance lane

System acceptance requires:

```text
sudo bash scripts/aletheon.sh deploy
```

The SHA-256 digest of `target/release/aletheon`, `/usr/bin/aletheon`, the
machine-core executable, and the user-daemon executable must match. Both units
must remain active with stable `NRestarts`, and the official socket must serve a
real model request.

## Known remaining gap

Retries currently honor exponential backoff and provider `Retry-After`, but
machine-wide concurrency and cooldown are not yet one authoritative mechanism
across main turns, multiple sessions, and external subagent runtimes. This must
be solved at the machine/provider boundary, not with prompt rules or
session-local hardcoded delays.

## Installed acceptance record: 2026-07-26

Source commit `e7150843c899027d361755c6fbe1b22dfda60023` was deployed with
`sudo bash scripts/aletheon.sh deploy`. The release artifact, installed client,
machine core, and user daemon all resolved to:

```text
ef8ab1ac8588b9ecfb9c387ee25e5eece9be5f7114ee1bd861bfb03b3ec27c7d
```

Both systemd units remained `active` with `NRestarts=0`. The deploy script's
official-client smoke request passed over the user socket.

The real TUI then completed a sustained three-turn challenge without losing the
host facts:

```text
effective_model_id = lejurobot_deepseek/deepseek/deepseek-v4-flash[1m]
display_name       = deepseek/deepseek-v4-flash[1m]
max_context_tokens = 1000000
```

Its session evidence is
`.scenario-runs/tui-events/12ce9882db2a480eb6e56d361d888f92.jsonl`
with three terminal turns. A later strict three-run sequence used the identical
runtime-fact prompt in fresh TUI sessions:

```text
.scenario-runs/tui-events/914dfba1c1a045a994c98fe6b94910ba.jsonl
.scenario-runs/tui-events/4e2a27a7ccfa4c1c872b1ef5c410340d.jsonl
.scenario-runs/tui-events/3090525f6c3844bfb4f4955d039c67b7.jsonl
```

Each file contains one terminal turn, each rendered frame returned `❯`, and
the journal interval beginning at `2026-07-26 02:42:05 CST` contained none of
the forbidden provider or infrastructure markers. The monitor reported a
settle timeout on the third capture even though its three retained final frames
were byte-identical, the prompt was visible, and the durable turn was terminal;
that is monitor timing evidence, not a model/runtime failure.
