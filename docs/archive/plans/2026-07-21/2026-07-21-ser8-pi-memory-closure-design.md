# SER8 Pi + Memory Closure Design

> Date: 2026-07-21
> Status: approved by the operator's standing instruction to use the recommended
> solution when deployment evidence is ambiguous.

## Requirement anchors

- The deployed system must run `core + user daemon`, use Leju DeepSeek, and
  close the scheduled Pi-to-memory-to-GBrain loop
  (`docs/archive/plans/2026-07-21/2026-07-21-aletheon-ser8-deployment.md:6-7`).
- Pi execution must be isolated, reviewed as a diff, settled, and persisted
  (`docs/archive/plans/2026-07-21/2026-07-21-aletheon-ser8-deployment.md:177-187`).
- GBrain is a shared local memory service and credentials must remain outside
  repository content (`docs/archive/plans/2026-07-21/2026-07-21-aletheon-ser8-deployment.md:16,184-185`).
- Scheduled operation must be a systemd user timer that submits a fixed task
  through the user daemon (`docs/archive/plans/2026-07-21/2026-07-21-aletheon-ser8-deployment.md:186-187`).

## Spec-versus-code adjudication

| Requirement description | Current code reality | Agree? |
|---|---|---|
| `pi-rpc` performs work in an isolated worktree and produces a reviewable diff (`deployment.md:181-183`) | Its runtime manifest declares `WorkspaceMode::Shared` (`crates/executive/src/impl/runtime/pi_rpc.rs:586-605`). | No |
| The Pi coding path creates and later collects a managed worktree (`deployment.md:181-183`) | `pi-coder` creates the lease, runs in it, collects the diff, and retains non-empty successful worktrees for approval (`crates/executive/src/impl/runtime/pi.rs:411-518,556-646`). | Yes, but the runtime ID in the document is wrong |
| Pi uses its independent cloud LLM while namespace-isolated (`deployment.md:104-107`) | `PiRuntime::prepare` rejects `network_enabled=true` (`crates/executive/src/impl/runtime/pi.rs:92-106`). | No for the deployed Leju endpoint |

Adjudication: preserve both existing runtime contracts instead of changing
`pi-rpc` into a second coding runtime. Use `pi-rpc` only for resident,
steerable interaction and use `pi-coder` for isolated changes and diff
settlement. This is the recommended low-blast-radius interpretation selected
under the operator's standing instruction.

## Considered approaches

1. **Dual runtime (selected):** `pi-rpc` remains resident/shared; `pi-coder`
   owns worktree/diff/apply. This follows the current manifests and avoids two
   competing worktree implementations.
2. **Make `pi-rpc` isolated:** add worktree ownership and settlement to the
   resident runtime. Rejected because it duplicates `pi-coder` and changes a
   published runtime contract.
3. **Use only `pi-coder`:** simplest isolation path, but loses the resident
   steering/follow-up capability explicitly provided by `pi-rpc`.

## Architecture

```text
systemd user timer
        |
        v
bounded submit wrapper ---> user daemon / Goal coordinator
                                  |
                   +--------------+--------------+
                   |                             |
                   v                             v
          pi-rpc (resident/shared)      pi-coder (one attempt)
                                             |
                                      bwrap + git worktree
                                             |
                                      bounded diff evidence
                                             |
                                  review -> approved_apply
                                             |
                                      settlement event
                                             |
                           Mnemosyne -> GBrain spool -> GBrain
```

### Runtime routing

- The main agent profile may invoke explicit Agent control operations but does
  not receive unrestricted shell or host access merely to orchestrate work.
- Conversational follow-ups explicitly select `runtime="pi-rpc"`.
- Repository modification attempts explicitly select `runtime="pi-coder"`,
  a pinned base commit, a disposable/managed worktree, bounded time/output,
  and reviewed apply. The first live test targets only a generated fixture.
- A failed or non-empty attempt is retained for evidence; it is never silently
  applied to the real Aletheon checkout.

### Network policy

- Default Pi network behavior remains deny.
- Enabling cloud inference is an explicit trusted configuration choice and is
  accepted only with namespace isolation, pinned Pi identity, credential
  injection through the systemd environment file, and bounded execution.
- Credentials are never placed in argv, prompts, evidence, logs, repository
  files, or timer units. Validation records only variable names and redacted
  presence.

### Memory path

- Terminal Agent settlement is represented as a `GoalOutcome`; raw user
  messages are not projected as durable GBrain pages.
- The same settlement is recorded locally before asynchronous GBrain delivery.
- GBrain unavailability queues a bounded spool entry; recovery drains it
  idempotently. Restart tests must prove local completion survives and the
  remote projection eventually appears once GBrain returns.

### Scheduled operation

- A user-owned wrapper uses a lock, hard timeout, fixed working directory,
  fixed prompt, and append-only private journal/evidence directory.
- The timer has randomized delay and persistence, but no overlapping runs.
- The wrapper addresses the existing user-daemon socket; it does not start an
  independent daemon and does not mutate system configuration.

## Failure handling

- Runtime registration, namespace probing, executable hash mismatch, unsafe
  diff, missing review receipt, or unavailable core fail closed.
- Provider/network failures retain bounded evidence and do not apply changes.
- GBrain failures do not roll back local settlement; they remain visible in
  the spool until a later successful drain.
- Timer failures are visible in the user journal and return non-zero without
  retry storms or concurrent executions.

## Verification gates

1. Three consecutive real-TUI turns return `turn_done`, a stable frame, and
   the input prompt, with forbidden infrastructure errors absent.
2. The pinned real Pi contract passes against `/usr/bin/pi` 0.80.10.
3. A disposable fixture run proves managed worktree creation, expected diff,
   settlement evidence, review/apply behavior, and an unchanged source
   checkout before approval.
4. Mnemosyne contains the terminal outcome and GBrain returns the projected
   canary without exposing credentials.
5. With GBrain stopped, a run settles locally and enters the spool; after
   restart it drains exactly once.
6. A manual timer invocation succeeds; overlap and timeout tests fail safely;
   unit and timer properties are captured in the acceptance record.

## Scope boundary

This closure does not redesign Pi's protocol, replace systemd, expose GBrain
outside localhost, or auto-apply changes to the Aletheon source checkout.
