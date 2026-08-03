# Governed Commands and Extension Runtime Design

## Purpose

Aletheon must expose only commands that require intentional user control. Its
reflection, evolution, evaluation, hook orchestration, and other governance
mechanisms must execute through typed internal lifecycle events rather than
text commands.

Aletheon must also finish its existing extension-package architecture so an
external project such as aurb can install governed Skills, Hooks, MCP
connectors, Agent Profiles, and executable Agent runtimes without copying files
into legacy directories or creating another source of truth.

## Current code reality

The following is the verified starting point for this design:

- The TUI registry currently exposes reflection, evolution, genome, hooks,
  evaluation, task selection, approval, plan, and computer commands alongside
  ordinary session commands (`crates/interact/src/tui/registry.rs:35-68`,
  `crates/interact/src/tui/registry.rs:264-413`).
- Those internal-facing entries are mapped into public `BuiltinCommand`
  variants (`crates/interact/src/tui/registry.rs:716-730`) and dispatched by the
  TUI submission path (`crates/interact/src/tui/app/submit.rs:138-170`,
  `crates/interact/src/tui/app/submit.rs:312-349`,
  `crates/interact/src/tui/app/submit.rs:434-466`).
- The CLI already supports extension inspection, validation, installation,
  listing, activation, upgrade, rollback, removal, purge, and diagnostics
  (`crates/aletheon/src/main.rs:332-379`,
  `crates/aletheon/src/main.rs:405-517`).
- Packages already declare Skill, Hook, Agent Profile, Connector, and
  Executable assets (`crates/fabric/src/types/extension_asset.rs:16-26`) with
  package-level permission requests
  (`crates/fabric/src/types/extension_package.rs:43-58`).
- Package inspection already validates archive paths, checksums, declared
  assets, and executable manifests
  (`crates/corpus/src/extension/inspector.rs:28-38`,
  `crates/corpus/src/extension/inspector.rs:102-181`).
- Activation state and permission approval are durable
  (`crates/corpus/src/extension/store.rs:81-98`,
  `crates/executive/src/application/extension_manage.rs:205-233`).
- At daemon bootstrap, package-backed runtime composition currently selects
  only `AssetKind::Executable`
  (`crates/executive/src/host/daemon/bootstrap/extensions.rs:183-208`).
- Skills and Hooks are still loaded from legacy fixed directories
  (`crates/executive/src/host/daemon/bootstrap/request.rs:496-528`), while MCP
  servers are sourced only from effective configuration
  (`crates/executive/src/host/daemon/bootstrap/request.rs:395-405`).

The package store is therefore the existing distribution and lifecycle
foundation, but it is not yet the sole runtime projection source for all asset
kinds.

## Goals

1. Remove system-governed internals from TUI help, completion, parsing, and
   externally triggerable client protocol paths.
2. Preserve reflection, evolution, evaluation, and hook behavior as internal,
   typed lifecycle processing with durable evidence.
3. Make the daemon the runtime authority for extension mutations and active
   extension snapshots.
4. Activate every supported package asset kind through one reconciler without
   copying package files into legacy user directories.
5. Allow aurb to build and install a versioned Aletheon package using the
   existing package contract.
6. Make activation, upgrade, disable, failure recovery, and restart behavior
   observable and deterministically testable.

## Non-goals

- Do not introduce an aurb-specific package store or plugin API.
- Do not retain hidden text aliases for removed governance commands.
- Do not permit extension packages to contain plaintext secrets.
- Do not hot-load unvalidated workspace files.
- Do not remove internal reflection, evolution, metacognition, audit, or
  extension evidence stores.
- Do not require every Skill, Hook, or MCP connector to be a separate package.

## 1. Public command surface

### Retained commands

The supported TUI command set is:

```text
/help       /new        /clear
/status     /sessions   /resume
/fork       /compact    /model
/permissions            /context
/interrupt  /copy       /mode
/quit       /agents     /agent
/skills     /profile    /diff
/mention    /input      /memory
```

These commands represent user-owned session control, visibility, selection,
content entry, or an explicit interruption/permission decision.

### Removed commands

The following entries and their aliases are removed from the TUI registry,
completion catalog, help rendering, parser mapping, submission dispatcher, and
public client command translation:

```text
/reflect       /reflect_now   /evolution
/genome        /hooks         /task
/evaluation    /approve       /plan
/computer
```

Removal means an input using one of these names is unknown; it must not silently
invoke a compatibility RPC. Test and diagnostic instrumentation must use typed
test APIs or host-level diagnostic commands, not ordinary chat input.

### Internal replacements

```text
turn/tool outcome
      │
      ├── reflection policy ──→ reflection evidence
      ├── evolution policy  ──→ proposal/experiment/settlement
      ├── evaluation policy ──→ typed evaluation receipt
      └── hook dispatcher   ──→ registered lifecycle hooks
```

- Reflection is triggered by typed turn/tool outcomes, corrections, failures,
  and configured periodic policy.
- Evolution is triggered only by the evolution coordinator and remains bounded
  by enablement, permission, evidence, and settlement policy.
- Genome state is internal state read by the evolution coordinator.
- Hook inventory becomes a section of `/status` and extension diagnostics,
  rather than a separately triggerable TUI operation.
- Coding task classification and evaluation remain typed client/test metadata,
  not slash commands.
- Approval uses the existing structured approval interaction.
- Plan behavior is selected through `/mode plan`; there is one mode-control
  entry point.

Read-only internal evidence remains available through structured diagnostics
and persisted receipts. Removing a TUI command must never remove auditability.

## 2. Extension runtime authority

### Selected approach

Use a daemon-owned `ExtensionCoordinator`. The CLI becomes a client of this
runtime authority for mutating operations. Read-only offline package inspection
and validation may remain local because they do not mutate installed or active
state.

```text
aletheon extension install/enable/upgrade/disable/rollback
                              │
                              │ authenticated user RPC
                              ▼
                    ExtensionCoordinator
                    ├── PackageInspector
                    ├── PackageStore
                    ├── ApprovalPort
                    ├── AssetResolver
                    ├── HealthProber
                    └── RuntimeSnapshot
```

Mutating the package store directly from an independent CLI process is retired
once the RPC path exists. This prevents the running daemon and CLI from holding
different views of active extensions.

### Runtime snapshot

The coordinator constructs an immutable candidate snapshot from enabled
activation records:

```text
ExtensionRuntimeSnapshot
├── skills
├── hooks
├── mcp_connectors
├── agent_profiles
├── agent_runtimes
├── package_digests
└── capability_digest
```

Package IDs, asset IDs, and asset paths are sorted before compilation. The
snapshot digest covers selected package hashes, activation state, resolved
asset descriptors, and relevant effective feature policy.

A candidate snapshot is published only after every asset validates and all
required health probes succeed. Readers receive either the complete old
snapshot or the complete new snapshot; they never observe a partial update.

## 3. Asset resolution

### Skill

- Resolve enabled `Skill` asset paths inside the package hash directory.
- Load each declared `SKILL.md` through the existing Skill parser.
- Register the package-qualified asset ID in the Skill catalog.
- Reject duplicate public names unless the package uses an explicit namespace.
- Feed the resolved catalog into both dynamic `skill_list`/`skill_get` tools and
  TUI `/skills` discovery.

### Hook

- Resolve enabled `Hook` manifests inside their package.
- Parse through the existing Hook loader contract.
- Require hook handlers and referenced payload files to remain inside the
  package, unless a separately approved executable runtime provides them.
- Register hooks only after validating event type, timeout, concurrency, and
  requested permissions.
- Unregister the complete package hook set atomically on disable or rollback.

### MCP Connector

- Treat the existing `Connector` asset kind as the package form of an MCP
  server definition for this phase.
- Define a versioned connector manifest containing transport, endpoint or
  subprocess runtime reference, tool/resource exposure policy, timeouts, and
  secret references.
- Resolve secret references through the host secret provider. A connector
  manifest cannot contain credential values.
- Merge package connectors and administrator-configured MCP servers by stable
  server ID. Conflicts fail closed; packages never override administrator
  configuration implicitly.
- Connect and enumerate tools before publishing the candidate snapshot.

### Agent Profile

- Load package Agent Profiles through the existing profile parser.
- Namespace profile identities by package unless explicitly declared public and
  conflict-free.
- Validate every allowed Tool, Skill, MCP capability, and runtime reference
  against the same candidate snapshot.
- A profile with an unresolved dependency blocks that package candidate.

### Executable Agent Runtime

Retain the current package-backed executable runtime, sandbox, permission, and
probe path. Move it behind the same candidate-snapshot transaction so its
publication and rollback semantics match the other asset kinds.

## 4. Lifecycle and failure semantics

```text
request
  → authenticate
  → inspect and checksum
  → stage package
  → request any permission elevation
  → persist installed version
  → compile candidate snapshot
  → probe candidate
      ├── success: publish snapshot + activation receipt
      └── failure: retain old snapshot + failure/quarantine receipt
```

- `install` validates and stores a package but does not activate it.
- `enable` activates an installed version after required approvals.
- `upgrade` stages and probes a candidate before replacing the active snapshot.
- `disable` publishes a snapshot without that package's assets.
- `rollback` probes previous-known-good before publishing it.
- Process failure after publication produces health evidence and invokes the
  existing governed recovery policy; it does not silently load workspace files.
- Daemon restart reconstructs the same snapshot from durable activation records
  and package hashes.
- During restart, one unhealthy package does not prevent the user daemon from
  starting. The coordinator first attempts its previous-known-good version;
  if that also fails, it quarantines that package and publishes a snapshot of
  the remaining healthy packages.

Each mutation returns a receipt containing operation, actor, package ID,
version/hash, previous and resulting snapshot digest, permission decision,
health result, and evidence references.

## 5. aurb packaging and installation

aurb integrates as a producer of the existing Aletheon package format:

```text
aurb sources
├── selected Skills
├── selected Hooks
├── MCP connector manifests
├── Agent Profiles
└── optional executable runtime payloads
       │
       ▼
aurb package aletheon
├── extension.toml
├── checksums.sha256
├── assets/...
└── payload/...
       │
       ▼
aurb-<version>.tar.gz
```

The initial aurb distribution is one package with multiple independently named
assets. This gives one coherent version and rollback boundary while preserving
asset-level diagnostics. Splitting into multiple packages is deferred until a
measured need for independent release or permission boundaries appears.

The aurb installer invokes public Aletheon CLI operations; it does not write
Package Store internals:

```text
aletheon extension validate <archive>
aletheon extension install <archive>
aletheon extension enable <package-id> --approve-permissions
aletheon extension doctor <package-id>
```

Workspace-produced archives continue to require explicit workspace trust. aurb
must expose that approval rather than bypassing it.

## 6. Compatibility and migration

- Existing enabled executable-runtime packages remain valid.
- Legacy filesystem Skills and Hooks remain readable during one compatibility
  window, but package assets take no precedence through path order. Any naming
  conflict fails closed and is reported.
- `extension import-legacy` remains the supported migration path into the
  Package Store.
- New aurb installation uses package mode only; it does not add more legacy
  directory content.
- Removed TUI commands are not kept as aliases. Release notes identify their
  automatic or consolidated replacement.

## 7. Validation and acceptance

### Command contracts

- Registry tests assert the retained command set exactly.
- Removed commands and aliases parse as unknown and never generate RPCs.
- Every retained command has a focused success and failure contract test.
- Help and completion contain only retained built-ins plus currently active
  daemon Skills.
- Reflection/evolution tests prove lifecycle events still trigger internal
  behavior without any text command.

### Extension contracts

- Deterministic fixtures package each asset kind individually and together.
- Install does not activate; enable atomically exposes all package assets.
- Skill discovery, Hook dispatch, MCP tool execution, Profile selection, and
  executable runtime execution are validated through authoritative daemon paths.
- Disable removes all package assets from the next snapshot.
- Upgrade failure retains the old snapshot; rollback restores previous-known-good.
- Restart reconstructs the same package and capability digests.
- Permission elevation and secret resolution fail closed.
- Concurrent mutations serialize at the coordinator/package boundary.

### aurb acceptance

- Build an aurb archive from the real aurb repository.
- Install and enable it with `/usr/bin/aletheon` through the official user
  socket.
- Observe an aurb Skill in the catalog, execute an aurb Hook from a real
  lifecycle event, and call an aurb-provided MCP tool.
- Disable the package and prove all three are absent.
- Re-enable, upgrade, and exercise rollback with durable receipts.

Because this changes client protocol, daemon bootstrap, extensions,
configuration, and runtime behavior, final acceptance requires
`sudo bash scripts/aletheon.sh deploy`, matching release/installed/running
binary digests, stable machine and user daemon restart counters, and a real LLM
request through `/usr/bin/aletheon` on the official socket.

## Implementation stages

1. **Command governance:** remove public internal commands and prove automatic
   lifecycle replacements.
2. **Protocol authority:** add daemon extension mutation/query RPCs and switch
   mutating CLI operations to them.
3. **Snapshot compiler:** introduce deterministic asset resolution and atomic
   runtime publication.
4. **Asset adapters:** integrate Skill, Hook, Connector/MCP, Profile, and the
   existing executable runtime.
5. **aurb producer:** build the existing package format from selected aurb
   assets and add install/upgrade verification.
6. **Migration and deployment:** validate legacy coexistence, real installed
   runtime behavior, recovery, and rollback.
