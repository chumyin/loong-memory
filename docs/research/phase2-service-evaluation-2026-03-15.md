# Phase 2 Service Evaluation (2026-03-15)

## Summary

This Phase 2 work now delivers both the initial daemon bootstrap and the first
transport-auth hardening pass for `loong-memoryd`, the standalone HTTP JSON
service that reuses the existing engine, policy, and audit contracts.

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

### 3. Optional Static Bearer Auth Envelope

The daemon now supports two explicit transport-auth modes:

- trusted-header mode when `--auth-file` is omitted
- static-token mode when `--auth-file` is present

Static-token mode requires `Authorization: Bearer <token>` and derives the
effective principal from the configured token mapping instead of trusting
caller-supplied `x-loong-principal`.

### 4. Per-Request Engine Construction

The core engine and SQLite store remain synchronous. Rather than sharing a
mutable engine in async handlers, the daemon creates a fresh engine per request
inside `spawn_blocking`. This avoids hot locks and keeps blocking work off the
reactor.

## Scope Delivered

- `crates/loong-memoryd`
- `GET /healthz`
- `POST /v1/memories`
- `DELETE /v1/memories`
- `POST /v1/memories/get`
- `POST /v1/recall`
- `POST /v1/audit`
- `POST /v1/vector-health`
- `POST /v1/vector-repair`
- structured HTTP error mapping
- startup config via `--db`, `--listen-addr`, `--policy-file`, `--auth-file`
- service-level negative-path coverage for malformed JSON, validation failure,
  and readiness failure
- updated repository docs

## Verification

Local verification passed:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`

Covered HTTP integration behaviors:

- health endpoint does not require a principal
- health endpoint reports both `policy_mode` and `auth_mode`
- health endpoint reports readiness failure when the configured DB path cannot
  be opened
- trusted-header mode rejects missing principal headers
- static-token mode accepts valid bearer tokens
- static-token mode rejects missing bearer tokens
- static-token mode rejects invalid bearer tokens
- static-token mode ignores spoofed `x-loong-principal` headers
- malformed JSON returns structured `invalid_json`
- put/get roundtrip succeeds over the daemon surface
- delete removes records over HTTP and preserves selector validation
- recall returns relevant results over HTTP
- invalid recall-weight requests return structured validation failures
- static policy denial returns `403`
- audit read returns namespace history without self-pollution
- vector health and vector repair are available over HTTP
- maintenance-route policy denial is preserved over HTTP

## Residual Gaps

This is a bootstrap slice, not the full Phase 2 end state. Still missing:

- gRPC transport
- SDK clients
- metrics/tracing export
- dynamic auth reload/rotation
- stronger external identity integration (OIDC, mTLS, or equivalent)

## Outcome

The repository now has a credible Phase 2 starting point: a real daemon
boundary, a real transport authentication envelope, structured health checks,
maintenance-route coverage, and end-to-end service tests for both happy and
negative paths. That materially improves delivery completeness over a CLI-only
Phase 1 story.
