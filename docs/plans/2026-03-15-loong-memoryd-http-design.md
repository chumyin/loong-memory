# loong-memoryd Minimal HTTP Service Design

Date: 2026-03-15
Owner: chumyin
Issue: #4

## 1. Context

`loong-memory` has completed a strong Phase 1:

- `MemoryEngine` already centralizes validation, policy checks, store access,
  and audit emission.
- the CLI can now load a static JSON policy file and route audit reads through
  engine-level authorization.
- SQLite durability, bounded recall, and audit persistence semantics are
  already covered by deterministic integration tests.

What the repository still lacks is the first Phase 2 transport/runtime surface
from the roadmap: a standalone daemon process that exposes the engine over a
service boundary.

## 2. Problem Statement

The project currently requires in-process Rust usage or direct CLI execution for
every memory operation. That keeps the engine reusable, but it prevents the
repository from demonstrating:

- a daemonized operational runtime
- a transport boundary for future SDK clients
- service-level health verification
- a concrete authn/authz envelope at the transport edge

Without a service slice, the roadmap's Phase 2 story is still conceptual rather
than delivered.

## 3. Goals

1. Add a standalone `loong-memoryd` binary to the workspace.
2. Expose a minimal HTTP JSON API that reuses existing engine semantics.
3. Require an explicit principal header for protected operations.
4. Start the daemon from a configured SQLite database path and optional static
   policy file.
5. Provide a structured health endpoint for operator and CI verification.
6. Add deterministic tests covering transport behavior and policy enforcement.
7. Update docs so repository status matches shipped behavior.

## 4. Non-Goals

- gRPC in this round
- Rust or TypeScript SDKs in this round
- dynamic policy reload
- bearer-token verification or external identity providers
- metrics, tracing export, or OpenTelemetry wiring
- maintenance endpoints for `vector_health` / `vector_repair`
- reworking store internals into async APIs

## 5. Approaches Considered

### Approach A: Full REST Surface with Resource-Oriented Routes

Expose canonical REST resources immediately, such as:

- `POST /v1/namespaces/:namespace/memories`
- `GET /v1/namespaces/:namespace/memories/:id`
- `GET /v1/namespaces/:namespace/audit`

Pros:

- closer to a polished public API
- cleaner URLs for long-term SDK generation

Cons:

- selector semantics (`id` XOR `external_id`) become awkward immediately
- more route and serialization work for a first service slice
- more design surface than needed to validate Phase 2 architecture

### Approach B: Minimal HTTP JSON Action Endpoints

Ship a small daemon with one health route and a few JSON endpoints that map
almost 1:1 onto existing engine request types.

Pros:

- fastest path to a real daemon
- minimal translation layer around `MemoryEngine`
- easy to test deterministically
- preserves current semantics without over-designing the public API

Cons:

- less REST-like
- likely not the final long-term public transport shape

### Approach C: Jump Straight to gRPC

Use protobuf contracts and a gRPC server as the first network transport.

Pros:

- strong future SDK/codegen story
- explicit schemas from the start

Cons:

- much larger dependency and tooling step
- slower path to the first working daemon
- higher design lock-in before validating runtime assumptions

## 6. Chosen Approach

Approach B.

This round should optimize for shipping the first production-shaped daemon
boundary, not for prematurely finalizing the entire external API style.

## 7. Proposed Runtime Shape

## 7.1 New Workspace Crate

Add a new crate:

- `crates/loong-memoryd`

The crate should contain:

- a library layer for service configuration, router construction, request
  handlers, and error mapping
- a binary entrypoint that parses CLI flags, initializes state, binds the TCP
  listener, and serves until shutdown

## 7.2 Configuration Model

The daemon should start from explicit CLI flags:

- `--db <path>`: SQLite database path, default `./loong-memory.db`
- `--listen-addr <addr>`: socket address, default `127.0.0.1:3000`
- `--policy-file <path>`: optional static JSON policy file

Runtime state should store:

- database path
- listen address
- loaded policy engine
- deterministic embedder

This keeps startup simple and avoids committing to a config-file format for the
daemon in the same round as the first transport slice.

## 7.3 Transport Surface

Expose the following endpoints:

- `GET /healthz`
- `POST /v1/memories`
- `POST /v1/memories/get`
- `POST /v1/recall`
- `POST /v1/audit`

Rationale:

- keeps scope tight
- covers write, point-read, ranked read, and audit-read semantics
- preserves current selector and recall request shapes cleanly

## 7.4 Principal Envelope

Protected routes must require:

- `x-loong-principal: <principal>`

Behavior:

- missing header returns `401 Unauthorized`
- blank header returns `401 Unauthorized`
- health endpoint does not require a principal

This is intentionally minimal. It is not presented as strong authentication,
only as the first transport identity envelope that maps onto the engine's
principal-aware policy model.

## 7.5 Request Execution Strategy

The core engine and SQLite layers are synchronous. Sharing a single mutable
engine in async handlers would force a hot lock and block reactor threads.

Instead, each protected request should:

1. clone immutable app state (`db_path`, `policy`, `embedder`)
2. enter `tokio::task::spawn_blocking`
3. create a fresh `SqliteStore`, `SqliteAuditSink`, and `MemoryEngine`
4. execute the requested engine operation
5. return the serialized result

Why this shape:

- avoids cross-request mutable state
- keeps synchronous SQLite work off async runtime workers
- reuses the same construction pattern already used by the CLI
- preserves current audit and policy semantics exactly

## 7.6 Error Model

Map engine and transport failures into structured JSON:

```json
{
  "error": {
    "code": "policy_denied",
    "message": "policy denied: ..."
  }
}
```

Initial mapping:

- validation -> `400 Bad Request`, `validation_failed`
- policy denied -> `403 Forbidden`, `policy_denied`
- not found -> `404 Not Found`, `not_found`
- missing principal -> `401 Unauthorized`, `missing_principal`
- unsupported method/route -> framework default `404`
- storage/internal/spawn failures -> `500 Internal Server Error`,
  `internal_error`

## 7.7 Response Shape

For operation endpoints, prefer direct JSON payloads over envelope-heavy
wrappers, except when a count is useful:

- `POST /v1/memories` -> `MemoryRecord`
- `POST /v1/memories/get` -> `MemoryRecord`
- `POST /v1/recall` -> `{ "count": N, "hits": [...] }`
- `POST /v1/audit` -> `{ "count": N, "events": [...] }`
- `GET /healthz` -> health object

Proposed health response:

```json
{
  "status": "ok",
  "service": "loong-memoryd",
  "db": "./loong-memory.db",
  "policy_mode": "allow_all"
}
```

If a policy file is supplied, `policy_mode` becomes `static`.

## 7.8 Health Semantics

`GET /healthz` should verify more than process liveness:

- open the configured SQLite store successfully
- open the configured SQLite audit sink successfully
- return structured JSON when initialization succeeds

If the database cannot be opened, the endpoint should fail with `500` and a
structured error payload. This keeps health checks honest.

## 7.9 Documentation Changes

Update:

- `README.md`
- `docs/architecture.md`
- `docs/roadmap.md`

The docs should describe:

- the new `loong-memoryd` crate and HTTP daemon surface
- minimal authn envelope via `x-loong-principal`
- health endpoint and example requests
- Phase 2 status as "started with minimal HTTP daemon surface"

## 8. Testing Strategy

## 8.1 Handler/Transport Tests

Add integration tests for:

- `GET /healthz` succeeds without principal
- protected endpoint rejects missing principal
- `POST /v1/memories` followed by `POST /v1/memories/get` succeeds
- `POST /v1/recall` returns relevant results
- static policy denial returns `403`
- `POST /v1/audit` returns existing namespace history and preserves current
  audit-read semantics

## 8.2 Daemon Runtime Test

Add at least one test that boots the router/server shape with an ephemeral
database and exercises the HTTP layer end to end. Use deterministic inputs and
keep the test local-only.

## 8.3 Verification Gates

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`

## 9. Risks and Mitigations

Risk: blocking SQLite work stalls async request handling.

Mitigation:

- execute engine/store work in `spawn_blocking`
- keep app state immutable and cheap to clone

Risk: transport code drifts from engine semantics.

Mitigation:

- keep endpoint request types close to existing model types
- route all protected operations through `MemoryEngine`
- add end-to-end tests for policy, audit, and recall behavior

Risk: the first HTTP surface is mistaken for a final public API.

Mitigation:

- document this explicitly as the minimal Phase 2 service slice
- avoid over-committing to REST or SDK generation in this round
