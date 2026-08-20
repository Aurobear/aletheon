# D6 unused UI model retirement

The D0 disposition ledger assigns the mixed Fabric UI-event model to owner
splits followed by D6 cleanup.
Exact Rust
caller searches found no caller outside the defining module/root re-export for
`EvolutionStage`, the rich `PlanUpdate` struct, or the obsolete
`SubAgentState` lifecycle machine. D6 therefore deletes those three rich
Fabric types rather than moving dead contracts into another owner.

The live wire `ClientEvent` and presentation models remain for separate owner
cutovers; this slice does not claim that the mixed module is closed.

The same exact caller-zero method retired five unused event payloads from the
adjacent mixed evolution module: `MutationIntentPayload`,
`EvolutionResultPayload`, `AgentStartedPayload`, `AgentStoppedPayload`, and
`AgentSpawnedPayload`. Their intended flows were never wired; moving them would
only preserve dead rich contracts.

Finally, the generic Fabric `BoundedStream`/`StreamSpec` utility and unused
`StreamEndReason` had no production caller; only their Fabric-local test kept
them alive. They were deleted without touching the live lossless turn-event
stream, whose `TurnEventStream`/`TurnEventSender` path remains independently
owned and validated.

The unused `BackgroundDisposition` request and
`HostDispositionAuthorization` wrapper were also definition/test-only. The
live Runtime settlement engine makes concrete kill/reparent decisions from
trusted resource and authority ports, so retaining an unwired detach request
would falsely advertise a second policy surface. D6 removed the dead pair and
its local-only test.

The generic `Subsystem::init_phase` hook had no override and no caller in the
workspace, so its `InitPhase` taxonomy could not affect composition order. D6
removed the dead hook and enum while retaining the live lifecycle methods.

`WorkspaceContent::ToolOutcome` likewise had no producer: only downstream
match arms remained. The governed tool-outcome variant is the live admitted
path, so D6 removed the unused frame and its dead consumer branches instead of
preserving a second, unauthorised outcome shape.

Two further definition-only surfaces were retired: unused Dasein
`SelfLineageV1`, and the old broad `CognitOps` service trait. The latter had no
production implementation or caller and was kept alive solely by the Fabric
mock-subsystem compile test; owner-specific Cognit ports already serve the live
runtime.

Three ContextSpace binding shapes (`MemoryViewId`, `WorldProjectionId`, and
`ProjectionVersion`) had no constructor or match anywhere outside their own
definition. D6 removed both the unused IDs and their never-instantiated enum
variants, leaving only the live Session, Agora, and Artifact bindings.

The legacy request `Context` trace and free-form metadata fields were never
read or written by a production caller, and its `child` helper was only used by
the Fabric-local mock test. D6 removed `TraceState`, the dead metadata API, and
the unused child-context mint instead of retaining a second correlation model.

The old `ActionResult.side_effects` list was written as empty at every
construction site and never read. Its `SideEffect`/`SideEffectKind` schema was
therefore non-functional duplicate evidence, not an extension contract. It was
removed; governed tool receipts remain the authoritative effect evidence.

## Additional ownerless/dead surface retirement

The D6 caller audit also proved that `HookMode`, `ModeConfig`, and `SafetyEvent`
had no production or test caller beyond their Fabric definitions/root re-exports. They were
never persisted or consumed as wire payloads, so the unused definitions and their governed
inventory rows were removed rather than migrated as artificial owner APIs.

The Runtime command/query census additionally found three declaration-only agent protocol
shapes (`AgentInteractionMode`, `AgentTaskEncoding`, and `AgentMessageReceipt`). No request,
response, persistence, test, or production path embedded them. D6 therefore deletes the
unshipped shapes instead of retaining misleading Runtime protocol surface.

Three obsolete Rust-only session protocol wrappers (`SessionProtocolV2` through V4) also had
no reader or writer; the retained JSON schemas remain the historical wire artifacts. The live
Rust protocol is V5. The declaration-only `Previewer` trait likewise had no implementation or
caller, so it was removed rather than carried into the owner contracts rename.

## Cognition evolution aggregate ownership

The remaining nine `events/evolution.rs` types were not ownerless contracts: all production
construction and scheduling lived in Cognit. D6 moved the aggregate intact to
`cognit::evolution`, changed Cognit's internal event observer/provider scheduler/pulse to its
owner module, and changed the Dasein mutation consumer and Executive rollback tests to the
owner API. The Fabric module and re-export were removed; no compatibility alias remains.

## UI presentation ownership

`AwarenessLevel`, `SubAgentStatus`, and `SubAgentHandle` were presentation state rather than
wire/domain contracts. D6 moved them to `interact::tui::presentation`. Cognit now keeps a
narrow four-state owner-local awareness projection and emits its existing string event fields;
Interact parses those strings into its presentation model. Fabric no longer exports the three
rich TUI types and no compatibility alias was added.

## Gateway ownership of legacy progress events

The remaining legacy daemon `ClientEvent` enum is a JSON notification protocol, not a Fabric
or TUI domain model. D6 moved it without wire-shape changes to
`gateway_protocol::legacy_progress::ClientEvent`; daemon formatters, the presentation decoder,
and rollback-only Executive sources now import the Gateway owner. Fabric's definition and
root re-export were deleted without an alias. This seam remains explicitly named `legacy_progress`
until CGP/XRET removes the old notification path in favor of typed `gateway_protocol::Event`.

## Turn-control policy ownership and event-shell retirement

`CollaborationMode` and `InterruptReason` are Application turn-control inputs, not UI/Fabric
models. They moved to `application::turn_control`; Cognit, Interact, Aletheon, and rollback-only
Executive callers use that owner API. The legacy Fabric RPC builder now accepts its actual wire
strings rather than importing an Application enum into Contracts. With all event payloads moved,
D6 deleted `fabric/src/events/ui_event.rs`, `events/mod.rs`, and the Fabric event module/re-exports.

## Metacognition genome and governance ownership

The Genome aggregate and `MetaRuntimeOps` candidate/evaluation/migration contracts had no
production owner outside Metacog. D6 moved the twelve Genome data types to
`metacog::genome::contracts` and the six runtime-governance types to
`metacog::governance::contracts`. Metacog implementation, contract tests, and the rollback-only
Executive evolution test now consume those owner paths. Fabric's `types/genome.rs`,
`include/meta.rs`, module declarations, root exports, and all eighteen census entries were
deleted; no compatibility alias remains.

## ACIX visual-grounding ownership

The grounding result, provider port, and test double are ACIX/Corpus contracts. D6 moved their
implementation to `corpus::acix::grounding`, removed Fabric's rich grounding module and three
public census rows, and deleted Interact's caller-zero compatibility namespace. The Interact
source census consequently moves from 53 to 52 files, matching the disposition ledger's
pre-existing DELETE decision for `interact/src/acix/mod.rs`.

The accompanying `Image`/`Bounds` value and PNG codec had the same sole production owner:
Corpus display/OCR/accessibility drivers. They moved to `corpus::drivers::types`, eliminating
Fabric's rich vision module and its two census rows without changing the ACIX API.

## Robot cognition perception ownership

`PerceptionObservation` is the bounded visual input to Cognit's robot harness and policy port.
D6 moved it to `cognit::harness::robot::perception`; the Dasein producer and host/test adapters
now depend on the Cognit-owned input contract. Fabric's module and census row were removed with
no alias.

## Corpus sandbox-glob adapter ownership

`expand_deny_globs` performs a bounded host-filesystem walk immediately before Corpus applies a
sandbox policy. It is adapter behavior rather than an ownerless contract. D6 moved the function
and its tests to `corpus::security::sandbox_glob`; the guarded runner now calls its local adapter.
Fabric retains only the pure policy, limits, and typed `ProfileResolveError` consumed at the
boundary. The Fabric module/root export and census row were removed without a compatibility alias.

## Turn-stream phantom overflow policy retirement

The canonical turn stream has used an unbounded lossless Tokio channel since the lifecycle-spine
fix, but `StreamConfig` still exposed four overflow policies and asserted at runtime that only
`BlockProducer` was accepted. D6 removed the false policy surface, the now-meaningless capacity
configuration, and the two unreachable `StreamSendError` variants. `TurnEventStream::new()` now
states the actual fixed transport contract directly: accepted events remain lossless until
receiver closure, which is the sole send error. No wire shape changed because these types are not
serialized.

## Corpus execution-policy ownership

The independent TOML/prefix/network policy parser and evaluator is a Corpus execution adapter,
not an ownerless Fabric contract. D6 moved the complete implementation and its contract tests to
`corpus::security::execpolicy`; `ToolRunnerWithGuard` now consumes its local policy API. The
unused RFC-017 `Decision` re-export, Fabric policy module/root exports, seven census rows, and old
Fabric tests were deleted without compatibility aliases. Kernel continues to receive only the
separate typed admission decision facts rather than parsing these rich rules.

## Corpus catalog authority contraction

The Fabric `ExtensionCatalog` trait had one implementation and no dynamic consumer; it only made
the Corpus catalog implement a trait owned by the former shared mega-crate. D6 deleted that
caller-zero port and made `snapshot` an inherent read operation on the Corpus-owned
`ExtensionCatalog`. Extension descriptor and snapshot wire shapes remain unchanged, so extension
preservation behavior is unaffected and no compatibility alias was introduced.

## Corpus extension metadata ownership

After the catalog authority contraction, every producer and consumer of the legacy extension
metadata contract flowed through Corpus catalog/activation APIs. D6 moved the seven remaining
identity, origin, activation, descriptor, snapshot, and validation types intact to
`corpus::catalog::contracts`, moved their wire-compatibility tests to Corpus, and changed the
Aletheon extension coordinator plus daemon bootstrap to the Corpus owner API. Fabric's module,
root exports, and census rows were deleted without aliases; JSON representations remain covered
by the relocated contract tests.

## Corpus hook-contract ownership and Runtime lifecycle split

The executable hook registry, loader, command envelope, and extension hook metadata are Corpus
capabilities, so D6 moved `HookPoint`, `HookContext`, `HookToolResult`, and `HookResult` intact to
`corpus::hook::contracts`. Runtime no longer imports those intervention contracts: supervised
child execution emits a narrow Runtime-owned `AgentLifecycleObservation`, and the outer Corpus
adapter maps its start/stop point and bounded identity metadata into a hook context. Fabric's hook
module, root exports, and four census rows were deleted without aliases; the relocated contract
and Corpus hook tests preserve serialization and execution behavior.

## Turn-stream send error contraction

The lossless turn stream's sender exposed a one-variant `StreamSendError` even though callers only
need success versus closed/serialization failure and never inspect that nominal type. D6 replaced
it with the unit error already consumed by every caller and removed the final phantom error type
from Fabric without an alias. The accepted-event lossless semantics and event wire schema remain
unchanged.

## Gateway schema wrapper retirement

`ClientProtocolSchema` existed only as an implementation detail passed to `schemars`; no production
caller constructed or named it. D6 removed that public wrapper and now builds the same request and
event schema document directly from the two public versioned message types. The schema function and
published request/event shapes remain covered by the protocol-schema contract test.

## Metacognition domain-id private validation error

`DomainIdError` had no caller outside `DomainId::new`; all consumers only require fail-closed
validation and format the returned error. D6 removed the public nominal error enum and returns
stable static validation messages instead. `DomainId` validation rules and its serialized form are
unchanged; the broader evaluation/evidence aggregate remains intact until it can move coherently.

## Agent settlement phantom detach authorization retirement

The earlier Runtime settlement cutover never called `authorize_background_disposition` and did not
model detached resources: authoritative policy settles foreground work, reparents qualified declared
survivors, and terminates everything else. D6 removed the caller-zero `BackgroundDisposition` and
`HostDispositionAuthorization` surface rather than preserve an unsupported detach mode. The active
Runtime settlement aggregate remains unchanged pending its coordinated split from Agent requests.

## Unused session JSON-RPC notification writer retirement

`session_notification_to_json` had no production, test, example, or installed route caller. The
active protocol uses typed event subscription and snapshot replay; retaining an unused writer for
`session.notification` would falsely advertise a second notification route. D6 deleted the helper
while retaining the versioned `SessionNotification` schema used by the checked-in session contract.

## Unused path and maintenance constants retirement

D6 removed two historical hook/snapshot path constants, two process-environment hook path helpers,
and an unenforced maintenance reason-code limit. None had a source, test, config, script, or
installed-route caller; retaining them would falsely advertise path authority and a validation
bound that the wire contract never enforced. Active `ProductionPaths`, Corpus hook resolution, and
Memory maintenance validation are unchanged.

## Internal validation limits made private

Six byte/count limits were implementation details used only by their defining Fabric validators;
no consumer imported them as protocol policy. D6 made the Memory query/kind/binding limits and
Agent output/artifact/broadcast limits private. Validation behavior and serialized contracts remain
unchanged, while the public facade no longer promises configuration knobs that callers cannot set.

## Retired duplicate Executive hook and plugin subsystems

Repository-wide caller audits found that Executive's old synchronous lifecycle-hook tree and its
plugin loader/manager/process runtime were reachable only through their own module declarations,
compatibility re-exports, and local unit tests. Production hook execution is owned by Corpus, while
extension discovery and activation use the typed extension path. D6 therefore removes both
caller-zero duplicate implementations and their compatibility exports instead of preserving second
hook and plugin-process authorities inside Executive.

The same audit found that the legacy `ContainerHost` had no binary, library, fixture, or test caller;
installed deployment is owned by Aletheon's wiring and system services. D6 removes that uncallable
host and its direct container-process authority rather than carry it into XRET-04.

Executive's alternate `SystemdHost` was likewise module-local and caller-zero. The installed
systemd lifecycle is selected by the Aletheon binary and its wiring, so the duplicate host was
removed without changing service units or the installed runtime path.

The retained Executive bootstrap also wrapped Corpus-owned tool and hook registries in a
field-only `CorpusGroup` with no behavior or boundary semantics. Its sole constructor and consumer
now inject the already-shared registry handles directly, removing another false owner layer.

The analogous `SecurityGroup` was not a security-domain contract either: it is a private bundle of
daemon bootstrap handles. It now lives beside the one bootstrap function that consumes it, removing
the Executive core's apparent ownership of Corpus's guarded runner without changing construction.

The final caller-zero chain through Executive's `DaemonHost` existed only to support the unused
legacy `host::daemon::run` wrapper. Removing that wrapper made `DaemonHost` and `RuntimeCore`
caller-zero as well, so both duplicate composition paths were deleted. The retained host module now
contains only fixtures/adapters still referenced by Aletheon or contract tests.

During XRET preparation, the closed-world Executive `adapter_registry` was found to be entirely
self-contained: no production, fixture, or integration test imported its IDs or classifier. It was
removed instead of carrying a second integration-selection taxonomy beside typed configuration.

XRET-04 caller audits also proved Executive's old host `launcher` and `doctor` modules had no
remaining caller. The Aletheon binary owns both live implementations, so the duplicate Executive
modules and their inventory rows were deleted.

Deleting the old host launcher made Executive's `SystemCoreRuntime` and provider wrapper
caller-zero. Machine inference composition already lives in `aletheon::wiring::core_runtime`; the
Executive duplicate and export were deleted.

Executive's readiness module also became caller-zero after the launcher deletion. Aletheon owns the
bounded, timed installed-runtime readiness implementation, so the old raw-process probe was removed
along with its obsolete kernel-effect census entry.

The default SelfField permission opinion is Dasein policy, not Executive orchestration. Its exact
threshold, capability comparison, verdict, and message moved to Dasein's existing permission port;
both composition paths now inject that owner implementation and the Executive wrapper was deleted.
