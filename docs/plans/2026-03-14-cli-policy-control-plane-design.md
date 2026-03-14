# Audit Guarantees and CLI Control Plane Design

Date: 2026-03-14
Owner: chumyin

## 1. Context

`loong-memory` already exposes policy and audit abstractions in the core engine,
and recent hardening work moved `vector-health` / `vector-repair` behind
engine-level authorization and audit boundaries. The current CLI still has an
important control-plane gap:

- `audit` reads directly through `SqliteAuditLog`, bypassing engine-level policy
  flow.
- `audit` allows omitting `namespace`, which conflicts with the documented
  security default that namespace is required in every operation.
- the CLI always uses `AllowAllPolicy`, so the repository claims
  principal-scoped/static policy support in practice only at the library layer,
  not at the default operator entrypoint.
- audit persistence is too important to remain a silent best-effort concern.

Evidence:

- `crates/loong-memory-cli/src/main.rs` routes `audit` directly to
  `SqliteAuditLog::list(...)`.
- `README.md` states "Policy is checked before store access" and "Namespace is
  required in every operation".
- `docs/architecture.md` states policy is enforced before every operation.

## 2. Problem Statement

The repository currently has a documented security model that is stronger than
the CLI control plane it actually ships. That makes the project look more
complete than it is and weakens the credibility of its policy boundary story.

This is not just a docs issue. The CLI is the operator surface for Phase 1, so
when it bypasses or weakens the control plane, the repository's real delivery
quality is lower than the architecture documents imply.

## 3. Goals

1. Make CLI policy enforcement operationally usable, not just theoretically
   available in `loong-memory-core`.
2. Route audit reads through the same authorization boundary as other protected
   operations.
3. Align CLI behavior with the documented namespace and audit model.
4. Surface audit persistence failures instead of silently dropping them.
5. Preserve current default ergonomics when no policy file is configured.
6. Add regression tests that prove the new control-plane behavior end to end.

## 4. Non-Goals

- dynamic RBAC / ABAC policy engines
- multi-namespace or global audit queries
- transport-layer authn/authz for a future daemon
- changing the storage format or retrieval algorithms
- fully atomic store+audit commit semantics in Phase 1

## 5. Approaches Considered

### Approach A: Narrow `audit` Command Hardening Only

Change only `audit` so it requires `namespace` and checks policy before reading.

Pros:

- smallest code change
- closes the direct audit-read bypass

Cons:

- still leaves the CLI stuck on `AllowAllPolicy`
- does not make principal-scoped policy usable in real operator flows
- keeps the gap between architecture claims and CLI reality only partially fixed

### Approach B: Delivery-Grade CLI Policy Control Plane + Audit Hardening

Add CLI-loadable static policy config, move audit reads behind an engine-level
audit read boundary, and harden audit persistence semantics.

Pros:

- closes the bypass
- makes static policy usable from the shipped binary
- aligns README, architecture, and operator behavior
- creates a reusable policy config path for future daemon/SDK layers
- makes audit loss/overwrite behavior explicit instead of silent

Cons:

- touches both core and CLI contracts
- requires new regression coverage and docs updates

### Approach C: Focus on Vector Integrity Preventive Hardening

Add stronger embedder/vector validation on write/query paths.

Pros:

- strong internal robustness value

Cons:

- does not address the most visible control-plane inconsistency
- lower delivery ROI for this phase than making the CLI security model real

## 6. Chosen Approach

Approach B.

This round should deepen the project where it most affects operator trust:
policy usability, audit persistence honesty, and audit-read governance at the
CLI boundary.

## 7. Proposed Design

## 7.1 Harden the Audit Contract

The audit contract should be fallible and append-only:

- audit write failures are surfaced
- SQLite audit rows reject duplicate `event_id` values
- namespace-scoped audit listing remains available for engine-level reads

Residual semantics:

- because store and audit are still separate in Phase 1, a post-operation audit
  failure can be returned after the store mutation has already committed

## 7.2 Extend the Audit Contract with Optional Read Support

Extend `AuditSink` in `loong-memory-core` so it can optionally expose
namespace-scoped audit listing:

- purpose: list audit events for a single namespace with bounded limits
- default behavior: explicit `NotImplemented`
- SQLite implementation: `SqliteAuditSink`

This keeps the existing engine construction path intact while allowing audit
read behavior only for sinks that actually support it.

## 7.3 Extend `MemoryEngine` with Audit Read Operation

Add an engine method for namespace-scoped audit reads:

- validates `namespace`
- validates/bounds `limit`
- enforces `Action::AuditRead`
- lists namespace audit events through the configured audit sink
- emits audit events summarizing the operation

Important implementation detail:

- The returned audit history must not be polluted by the audit command's own
  authorization event. To preserve useful results, the audit read operation
  should query the log first and emit its allow/read events only after the read
  result has been collected.

This requires a small refactor away from using the generic `enforce()` helper
for audit reads, because the allow event must be emitted only after the history
has been collected.

## 7.4 Make Static Policy Config Loadable from the CLI

Add a CLI-wide optional `--policy-file <path>` flag.

Behavior:

- when absent: preserve current `AllowAllPolicy` default
- when present: load JSON config into `StaticPolicy`
- malformed config: fail fast with path-aware error text

Proposed JSON schema:

```json
{
  "namespace_actions": {
    "shared-readonly": ["get", "recall"]
  },
  "principal_namespace_actions": [
    {
      "principal": "operator",
      "namespace": "agent-demo",
      "actions": ["put", "get", "recall", "delete", "audit_read", "repair"]
    }
  ]
}
```

Why JSON:

- already supported by existing dependencies
- easy to validate and document
- avoids adding a new parser dependency for this phase

## 7.5 Tighten CLI Audit Semantics

Change `loong-memory audit` to require:

- `--namespace`
- `--principal`

And route it through `MemoryEngine`, not direct `SqliteAuditLog`.

This makes audit inspection consistent with the repository's existing security
model:

- namespace-scoped
- policy-gated
- auditable

## 7.6 Documentation and Example Artifacts

Update:

- `README.md`
- `docs/architecture.md`
- a new example policy file under `docs/examples/`
- a research note for this round under `docs/research/`

The README should explicitly state:

- CLI defaults to allow-all unless `--policy-file` is supplied
- namespace-scoped audit reads now require `--namespace` and `--principal`
- example policy grants for operator maintenance flows

## 8. Testing Strategy

### Core Tests

- policy config parsing into `StaticPolicy`
- engine audit read is denied without `Action::AuditRead`
- engine audit read succeeds with `Action::AuditRead`
- audit read without configured reader returns `NotImplemented`
- returned audit history does not contain self-generated audit-read events

### CLI Tests

- `audit` requires `--namespace`
- policy file can authorize `put` + `audit`
- policy file denial prevents `audit`

### Verification Gates

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`

## 9. Risks and Mitigations

Risk: audit-read self-pollution makes small-limit results unusable.

Mitigation:

- list first, emit audit events after collecting results
- add a regression test for this exact behavior

Risk: CLI policy file format becomes too clever for Phase 1.

Mitigation:

- JSON only
- no inheritance, wildcards, or condition expressions
- direct mapping to existing `StaticPolicy`

Risk: engine constructor churn spreads unnecessary change.

Mitigation:

- keep existing constructor path viable
- attach audit-read capability through the existing audit sink abstraction

## 10. Success Criteria

- a user can enforce namespace/principal-scoped permissions from the CLI with a
  checked-in JSON policy file
- `audit` no longer bypasses the engine control plane
- README and architecture docs match shipped CLI behavior
- regression tests prove both allow and deny behavior
