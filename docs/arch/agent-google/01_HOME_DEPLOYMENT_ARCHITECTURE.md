# Home Deployment Architecture

> **Status:** Deployment target; current code capabilities are explicitly noted
>
> **Rewritten:** 2026-07-19

## Deployment Unit

Aletheon is deployed as one supervised application instance plus optional
isolated child processes:

```text
service manager
    `-- aletheon daemon
          |-- local durable stores
          |-- channel workers
          |-- optional execd
          `-- supervised external Runtime sessions
```

Do not deploy each domain crate as a separate service. Crates are internal
ownership boundaries, not network topology.

## Process Boundaries

- `aletheon daemon` is the composition root and authority owner.
- `execd` is optional and isolated because it performs low-level file/process
  side effects; Executive launches it from
  `crates/executive/src/impl/daemon/bootstrap/request.rs:452-479`.
- External Runtime or robot-edge processes are supervised execution domains,
  not alternative Executive authorities.

## Files and Storage

Deployment must provide distinct locations for:

```text
configuration       operator-controlled, backed up without secrets in logs
credentials         least privilege, restrictive filesystem permissions
durable state       event/goal/session stores with recovery testing
workspace           explicit root, never inferred from a developer path
artifacts            bounded retention and integrity metadata
logs                 rotation, redaction and bounded disk usage
```

The application root schema is defined at
`crates/executive/src/core/config/mod.rs:64-85`. Configuration selects features;
it does not itself grant runtime permission.

## Network Exposure

Default deployment is local-first:

- daemon IPC stays local unless an authenticated gateway is deliberately added;
- Telegram and Google use outbound provider connections;
- remote administration requires an authenticated transport and must not expose
  raw daemon or `execd` stdio;
- robot control crosses a Hardware provider and device-local safety authority.

## Supervision and Recovery

The service manager owns daemon restart. Inside Aletheon, Kernel owns Operation
and Process lifecycle. Restart acceptance requires:

1. no orphaned `execd` children;
2. durable goals and sessions reopen deterministically;
3. expired leases remain expired;
4. pending approvals do not silently become approved;
5. Runtime recovery never allocates a second authority for the same work.

## Backup

Back up durable stores, configuration and required artifacts. Do not treat
caches or projections as authorities. Recovery tests must restore to a fresh
machine and verify schema compatibility, not merely copy files in place.

## Minimum Acceptance

- daemon starts from a non-repository working directory;
- configuration and secret permissions are checked;
- clean stop and forced termination leave no child processes;
- restart restores durable goals/sessions without duplicate execution;
- backup and restore are exercised;
- disabled optional integrations are reported, not silently simulated.
