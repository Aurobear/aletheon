# GBrain Memory Source Grant Introspection Implementation Plan

> **For agentic workers:** Use `flow-feature` or `plans` to execute this plan task by task; preserve the TDD order and commit the GBrain change independently from Aletheon integration.

**Goal:** Make GBrain's authenticated `whoami` response expose a stable, token-free, fail-closed description of the caller's effective write Source and federated read Sources so Aletheon can verify workspace bindings before remote projection.

**Architecture:** GBrain remains the authority for token grants. The existing HTTP transport continues to populate `AuthInfo.sourceId` and `AuthInfo.allowedSources`; `whoami` only normalizes those effective grants into an additive schema. Local callers receive an explicit unscoped shape, and remote calls without authentication keep failing closed. Aletheon will consume this contract in a later adapter plan; this slice does not add destination selection or credential storage to GBrain.

**Tech Stack:** TypeScript, Bun test runner, GBrain operation dispatch, OAuth/legacy bearer authentication.

---

## Requirement and code anchors

- The approved design requires source-bound credential selection and verified grants, not a caller-supplied destination (`docs/plans/2026-08-01-unified-memory-gateway-design.md:section 4.3`).
- `whoami` must expose `source_scope_schema`, `write_source`, and `read_sources` without exposing a token (`docs/plans/2026-08-01-unified-memory-gateway-design.md:section 4.3`).
- GBrain already threads write and read grants through `AuthInfo.sourceId` and `AuthInfo.allowedSources` (`/home/aurobear/Workspace/agent/gbrain/src/core/operations.ts:231-275`).
- The current `whoami` response omits both grants (`/home/aurobear/Workspace/agent/gbrain/src/core/operations.ts:3676-3724`).
- Pure contract coverage is in `/home/aurobear/Workspace/agent/gbrain/test/whoami.test.ts:29-135`; real HTTP coverage is in `/home/aurobear/Workspace/agent/gbrain/test/e2e/sources-remote-mcp.test.ts:236-241`.

## Contract

Every successful `whoami` response adds:

```json
{
  "source_scope_schema": 1,
  "write_source": "workspace-source-or-null",
  "read_sources": ["sorted", "unique", "effective", "sources"]
}
```

Rules:

1. `write_source` is exactly the authenticated `sourceId`, or `null` when none is granted.
2. A non-empty `allowedSources` is the authoritative read set after deduplication and stable sorting.
3. If `allowedSources` is absent or empty and `sourceId` exists, the effective read set is `[sourceId]`, matching GBrain's documented scalar fallback.
4. If neither grant exists, the response is fail-closed: `write_source: null`, `read_sources: []`.
5. Local transport always returns the unscoped shape and ignores stale `auth` data.
6. No response field contains the bearer token.

### Task 1: Pin the source-grant contract with failing tests

**Files:**
- Modify: `/home/aurobear/Workspace/agent/gbrain/test/whoami.test.ts`
- Modify: `/home/aurobear/Workspace/agent/gbrain/test/e2e/sources-remote-mcp.test.ts`

- [ ] Add assertions that local transport returns schema `1`, a null write Source, and an empty read set.
- [ ] Extend the stale-local-auth case with source grants and assert they are not reflected.
- [ ] Add an OAuth case with duplicated, unsorted `allowedSources`; assert exact sorted/deduplicated output and unchanged identity fields.
- [ ] Add an OAuth scalar-fallback case with `sourceId` and no federated set; assert `read_sources` contains only the write Source.
- [ ] Add an unscoped authenticated OAuth case; assert null/empty rather than `default`.
- [ ] Add a legacy bearer case with `sourceId` and `allowedSources`; assert the same normalized contract.
- [ ] Assert serialized results do not contain the supplied bearer token.
- [ ] Extend the existing real HTTP `whoami` assertion. Its registered client defaults to Source `default`, so assert `write_source === "default"`, `read_sources === ["default"]`, and schema `1`.
- [ ] Run `bun test test/whoami.test.ts` from the GBrain repository and confirm the new assertions fail before production code changes.

### Task 2: Normalize effective grants in `whoami`

**Files:**
- Modify: `/home/aurobear/Workspace/agent/gbrain/src/core/operations.ts`

- [ ] Add a small pure helper adjacent to `whoami` that returns the three contract fields from optional `AuthInfo`.
- [ ] Deduplicate and sort a copied `allowedSources` array; never mutate authentication context.
- [ ] Apply scalar read fallback only when `allowedSources` is absent or empty and `sourceId` exists.
- [ ] Merge the unscoped helper result into the local response before inspecting any stale `auth` value.
- [ ] Merge the authenticated helper result into both OAuth and legacy response shapes.
- [ ] Update operation metadata/description so generated tool documentation advertises the additive source-scope contract.
- [ ] Run `bun test test/whoami.test.ts` and confirm all focused contract tests pass.

### Task 3: Verify transport and static contracts

**Files:**
- Verify only: `/home/aurobear/Workspace/agent/gbrain/src/mcp/http-transport.ts`
- Verify only: `/home/aurobear/Workspace/agent/gbrain/src/core/operations.ts`

- [ ] Run `bun run typecheck`.
- [ ] Run `bun run check:source-config-leak` to ensure the response does not introduce forbidden source configuration fallbacks.
- [ ] If `GBRAIN_DATABASE_URL` or `DATABASE_URL` is configured, run `bun test test/e2e/sources-remote-mcp.test.ts`; otherwise record the database-dependent skip and rely on the pure contract test plus typecheck.
- [ ] Inspect `git diff --check` and the complete GBrain diff.
- [ ] Commit the GBrain change with a conventional subject and a body explaining the missing grant-verification contract and the normalized, token-free solution.

## Acceptance

- An authenticated caller can distinguish an unscoped credential from a Source-bound credential using only server-reported effective grants.
- Read grants are deterministic, unique, and reflect federated-read precedence with scalar fallback.
- Local and unscoped responses do not manufacture `default` authority.
- Existing OAuth/legacy/local identity fields remain backward compatible.
- Unknown remote transports still fail closed.
- No token value appears in any response.
