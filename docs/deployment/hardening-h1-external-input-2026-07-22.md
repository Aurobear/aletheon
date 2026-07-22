# Production hardening H1 external-input evidence — 2026-07-22

## Requirement

H1 is defined at
`docs/plans/2026-07-21-production-readiness-hardening.md:113-133`: locate a
panic reachable from untrusted external or persisted input, reproduce it before
editing, convert it to a structured rejection, and prove no partial state or
provider download occurs.

## Reproduced failure

A Gmail MIME part is considered an attachment when it has either a filename or
an attachment ID (`crates/executive/src/impl/channel/gmail/ingest.rs:308-310`).
Before this batch, `attachment_rejection` returned `None` immediately when the
filename was absent. A remote part with an attachment ID, no filename, and no
declared size therefore bypassed rejection and reached:

```text
crates/executive/src/impl/channel/gmail/ingest.rs:169
part.declared_size.expect("checked by rejection")
```

The regression test was added and run before the implementation change. It
failed with:

```text
panicked at crates/executive/src/impl/channel/gmail/ingest.rs:169:47:
checked by rejection
```

Input shape:

```text
attachment_id = Some("attachment-nameless")
filename = None
declared_size = None
mime_type = "text/plain"
```

This is provider-controlled Gmail MIME metadata and therefore satisfies the H1
external-input P0 threshold.

## Fix

`attachment_rejection` now classifies a missing filename as
`missing_filename` before size or attachment-ID assumptions are read. The
existing unavailable-evidence path records the rejection, and the attachment
fetcher is never called. No Goal/session or artifact is created by this stage.

The other initially named unwrap/expect clusters were reviewed:

- Gmail Goal draft value access follows `validate_goal_ingress`, which checks
  principal, sender and policy version before the guarded values are used
  (`crates/executive/src/impl/channel/gmail/goal_draft.rs:510-566`);
- GBrain adapter expects are mutex-poisoning invariants, while malformed remote
  response handling returns typed categories
  (`crates/executive/src/impl/gbrain/mcp_adapter.rs:18-55,134-145`);
- daemon adapter production unwraps are repository/coordinator mutex poisoning,
  not direct malformed-message parsing
  (`crates/executive/src/impl/channel/daemon_adapter.rs:48-90,301-385`).

They were not mechanically changed in H1.

## Verification

```bash
bash scripts/cargo-agent.sh test -p executive --test gmail_attachment_ingest \
  attachment_without_filename_is_rejected_without_panicking_or_downloading -- --exact
bash scripts/cargo-agent.sh test -p executive --test gmail_attachment_ingest
bash scripts/cargo-agent.sh fmt --all -- --check
git diff --check
```

Results:

- targeted regression: PASS;
- full `gmail_attachment_ingest` target: PASS, 5 tests;
- formatting and whitespace checks: PASS;
- rejection reason: `missing_filename`;
- provider download calls: zero.

The requested clippy command was also attempted. Both with and without
`--no-deps`, `-D warnings` is currently blocked by pre-existing
`clippy::uninlined_format_args` findings across Fabric and Executive, unrelated
to the two changed Rust files. No unrelated bulk lint cleanup was folded into
H1.

## Disposition

H1 is complete: one provider-controlled malformed input panic was reproduced,
fixed through structured rejection, and covered by a deterministic regression.
H2 Provider single-source work may start.
