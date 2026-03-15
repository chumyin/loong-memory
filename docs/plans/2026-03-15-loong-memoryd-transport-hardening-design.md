# loong-memoryd Transport Hardening Design

Date: 2026-03-15
Owner: chumyin
Issue: #6

## 1. Context

`loong-memoryd` now provides the first real Phase 2 daemon surface:

- `GET /healthz`
- `POST /v1/memories`
- `POST /v1/memories/get`
- `POST /v1/recall`
- `POST /v1/audit`

The transport layer already preserves the core engine's validation, policy, and
audit semantics by constructing a fresh `MemoryEngine` per request inside
`spawn_blocking`. The next delivery gap is no longer "is there a daemon at
all?", but "does the daemon expose enough of the engine's operational surface to
be credible for real service use?"

## 2. Problem Statement

The engine and CLI already support:

- delete
- vector health diagnostics
- vector repair planning and apply mode
- detailed validation and policy failures

The daemon does not yet expose those capabilities over HTTP, and its service
tests still focus mainly on happy paths. That leaves a gap between the shipped
daemon surface and the already-delivered operational capabilities underneath it.

## 3. Goals

1. Add HTTP delete support to `loong-memoryd`.
2. Add HTTP `vector_health` support to `loong-memoryd`.
3. Add HTTP `vector_repair` support to `loong-memoryd`.
4. Preserve existing engine policy and audit semantics for all new routes.
5. Add service-level negative-path regression tests for malformed JSON and
   validation failures.
6. Add a readiness-failure regression test for `/healthz`.
7. Update docs and research notes to reflect the broader daemon surface.

## 4. Non-Goals

- gRPC transport
- SDK clients
- auth provider integration
- policy hot reload
- shared engine pooling or latency optimization
- daemon-side metrics/tracing export

## 5. Approaches Considered

### Approach A: Negative Tests Only

Keep the route surface unchanged and only add malformed-input and readiness
failure coverage.

Pros:

- smallest code change
- strengthens transport error confidence

Cons:

- still leaves delete and maintenance capabilities missing from the daemon
- keeps the transport surface visibly behind the CLI

### Approach B: Transport Hardening Slice

Add delete + maintenance routes and pair them with stronger negative-path
coverage.

Pros:

- closes the most obvious daemon capability gaps
- keeps scope within the current HTTP design
- reuses existing engine operations with low architectural risk
- materially improves Phase 2 completeness

Cons:

- touches both handlers and integration tests
- expands docs again immediately after the first daemon release

### Approach C: Jump to gRPC / SDK Work

Leave the current daemon shape as-is and move to new protocol or client layers.

Pros:

- advances later Phase 2 roadmap items

Cons:

- compounds an incomplete service surface instead of stabilizing it
- risks freezing awkward API gaps into future client contracts

## 6. Chosen Approach

Approach B.

The best next move is to harden the existing daemon before layering more
transport or client abstractions on top of it.

## 7. Proposed Design

## 7.1 Route Additions

Add the following endpoints:

- `DELETE /v1/memories`
- `POST /v1/vector-health`
- `POST /v1/vector-repair`

All three remain protected by `x-loong-principal`.

## 7.2 Delete Transport Shape

Use:

- `DELETE /v1/memories`

Request body:

```json
{
  "namespace": "agent-demo",
  "external_id": "profile"
}
```

The handler maps directly to `MemoryDeleteRequest`.

Why this shape:

- keeps route naming consistent with the existing `POST /v1/memories`
- preserves current selector semantics (`id` XOR `external_id`)
- avoids premature resource-URL design churn

Success response:

```json
{ "ok": true }
```

## 7.3 Maintenance Transport Shape

`vector_health` request body:

```json
{
  "namespace": "agent-demo",
  "invalid_sample_limit": 20
}
```

`vector_repair` request body:

```json
{
  "namespace": "agent-demo",
  "issue_sample_limit": 20,
  "apply": false
}
```

These map directly onto the existing engine methods:

- `MemoryEngine::vector_health`
- `MemoryEngine::vector_repair`

This is important because the engine already owns the correct policy and audit
behavior:

- `vector_health` is gated by `Action::AuditRead`
- `vector_repair` is gated by `Action::Repair`

## 7.4 Error Semantics

Retain the current structured error envelope and strengthen tests around it.

Important negative-path cases to pin at the HTTP layer:

- malformed JSON -> `400`, `invalid_json`
- invalid recall weights -> `400`, `validation_failed`
- invalid delete selector (`id` + `external_id`) -> `400`, `validation_failed`
- missing principal -> `401`, `missing_principal`
- policy denial -> `403`, `policy_denied`
- readiness failure -> `500`, `internal_error`

The core engine already generates most validation and policy failures; the goal
here is to prove the transport preserves them.

## 7.5 Readiness Failure Test

`/healthz` should stay honest. Add a test that starts the daemon with a database
path nested under a non-existent directory so SQLite open fails at health-check
time.

Expected behavior:

- server starts
- `GET /healthz` returns `500`
- response uses the structured error envelope

This is preferable to trying to force the daemon to fail at startup, because it
tests the documented readiness semantics directly.

## 7.6 Test Strategy

Add or extend HTTP integration tests for:

- `DELETE /v1/memories` happy path
- invalid delete selector
- `POST /v1/vector-health` happy path
- `POST /v1/vector-repair` dry-run happy path
- policy denial for at least one maintenance route
- malformed JSON on a protected endpoint
- invalid recall-weight validation
- `/healthz` readiness failure

Keep tests end to end through the live HTTP server helper.

## 7.7 Documentation Updates

Update:

- `README.md`
- `docs/architecture.md`
- `docs/roadmap.md` if needed only for wording clarity
- `docs/research/phase2-service-evaluation-2026-03-15.md`

The README should show the expanded daemon route set and example requests for
delete and maintenance operations.

## 8. Risks and Mitigations

Risk: HTTP delete with JSON body is less canonical than path-based REST design.

Mitigation:

- keep the route intentionally minimal and document it as part of the Phase 2
  bootstrap surface
- preserve the engine's established selector contract instead of inventing a
  more rigid URL layout too early

Risk: maintenance routes over-expose privileged functionality.

Mitigation:

- route only through existing engine methods
- rely on the same policy and audit semantics already exercised by the CLI
- add deny-path tests at the service layer

Risk: transport error mapping drifts from current behavior.

Mitigation:

- pin negative cases with integration tests instead of only unit-level mapping
  logic
