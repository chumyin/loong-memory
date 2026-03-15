# Phase 2 Service Evaluation (2026-03-15)

## Summary

This round delivers the first Phase 2 runtime boundary for `loong-memory`:
`loong-memoryd`, a standalone HTTP JSON daemon that reuses the existing engine,
policy, and audit contracts.

## Why This Was the Right Next Step

Before this change, the repository already had a solid engine and CLI, but it
still lacked any transport boundary. That meant:

- no daemonized runtime
- no service-level health verification
- no concrete transport principal envelope
- no direct path toward future SDK clients

Adding the smallest real daemon slice closed the largest remaining delivery gap
without overcommitting to a final public API shape.

## Design Decisions

### 1. HTTP JSON Before gRPC

HTTP JSON was chosen as the first transport because it is the fastest path to a
production-shaped daemon with low integration overhead. gRPC remains a later
Phase 2 expansion, not a blocker for validating the runtime boundary.

### 2. Thin Transport Layer

Handlers delegate to `MemoryEngine` instead of re-implementing validation,
policy, or audit logic. This keeps the transport layer small and preserves the
repository's existing trust boundary.

### 3. Explicit Principal Header

Protected routes require `x-loong-principal`. This is intentionally minimal and
maps directly onto the existing engine policy model without pretending to be a
complete authentication solution.

### 4. Per-Request Engine Construction

The core engine and SQLite store remain synchronous. Rather than sharing a
mutable engine in async handlers, the daemon creates a fresh engine per request
inside `spawn_blocking`. This avoids hot locks and keeps blocking work off the
reactor.

## Scope Delivered

- `crates/loong-memoryd`
- `GET /healthz`
- `POST /v1/memories`
- `POST /v1/memories/get`
- `POST /v1/recall`
- `POST /v1/audit`
- structured HTTP error mapping
- startup config via `--db`, `--listen-addr`, `--policy-file`
- updated repository docs

## Verification

Local verification passed:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`

Covered HTTP integration behaviors:

- health endpoint does not require a principal
- protected routes reject missing principal headers
- put/get roundtrip succeeds over the daemon surface
- recall returns relevant results over HTTP
- static policy denial returns `403`
- audit read returns namespace history without self-pollution

## Residual Gaps

This is a bootstrap slice, not the full Phase 2 end state. Still missing:

- delete and maintenance HTTP routes
- gRPC transport
- SDK clients
- metrics/tracing export
- stronger transport authentication and policy reload

## Outcome

The repository now has a credible Phase 2 starting point: a real daemon
boundary, transport-level identity propagation, structured health checks, and
end-to-end service tests. That materially improves delivery completeness over a
CLI-only Phase 1 story.
