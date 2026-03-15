# loong-memoryd Minimal HTTP Service Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a minimal `loong-memoryd` daemon that exposes health, put, get, recall, and audit operations over HTTP JSON while preserving the existing engine, policy, and audit semantics.

**Architecture:** Add a new `crates/loong-memoryd` workspace crate built on `axum` + `tokio`. Keep app state immutable, require `x-loong-principal` for protected routes, and run synchronous SQLite/engine work inside `spawn_blocking` with per-request engine construction.

**Tech Stack:** Rust, `axum`, `tokio`, `serde`, `tower`, `reqwest`, existing `loong-memory-core`

---

### Task 1: Add the new daemon crate skeleton

**Files:**
- Modify: `Cargo.toml`
- Create: `crates/loong-memoryd/Cargo.toml`
- Create: `crates/loong-memoryd/src/lib.rs`
- Create: `crates/loong-memoryd/src/main.rs`

**Step 1: Write the failing test**

Create an integration test module placeholder under `crates/loong-memoryd/tests/http_service_integration.rs` that imports the future crate and fails because the crate/router does not exist yet.

**Step 2: Run test to verify it fails**

Run: `cargo test -p loong-memoryd`
Expected: FAIL because the package or crate has not been added yet.

**Step 3: Write minimal implementation**

- add the crate to the workspace
- declare dependencies for `axum`, `tokio`, `serde`, `serde_json`, `anyhow`, `thiserror`, `tower`, and `reqwest`
- add a stub library with exported config/state placeholders
- add a stub binary that parses CLI flags and exits cleanly

**Step 4: Run test to verify it passes**

Run: `cargo test -p loong-memoryd`
Expected: PASS for the crate skeleton with placeholder coverage.

**Step 5: Commit**

```bash
git add Cargo.toml crates/loong-memoryd
git commit -m "feat: add loong-memoryd crate skeleton"
```

### Task 2: Add health endpoint and structured error model

**Files:**
- Modify: `crates/loong-memoryd/src/lib.rs`
- Modify: `crates/loong-memoryd/src/main.rs`
- Modify: `crates/loong-memoryd/tests/http_service_integration.rs`

**Step 1: Write the failing test**

Add tests covering:

- `GET /healthz` returns `200`
- response JSON includes `status`, `service`, and `policy_mode`

**Step 2: Run test to verify it fails**

Run: `cargo test -p loong-memoryd healthz`
Expected: FAIL because no router/health handler exists.

**Step 3: Write minimal implementation**

- add `ServiceConfig`, `PolicyMode`, and immutable app state
- add router construction with `GET /healthz`
- open SQLite store/audit in the health handler to ensure honest readiness
- add structured JSON error responses and HTTP status mapping helpers

**Step 4: Run test to verify it passes**

Run: `cargo test -p loong-memoryd healthz`
Expected: PASS.

**Step 5: Commit**

```bash
git add crates/loong-memoryd/src/lib.rs crates/loong-memoryd/src/main.rs crates/loong-memoryd/tests/http_service_integration.rs
git commit -m "feat: add loong-memoryd health endpoint"
```

### Task 3: Enforce principal header and request helpers

**Files:**
- Modify: `crates/loong-memoryd/src/lib.rs`
- Modify: `crates/loong-memoryd/tests/http_service_integration.rs`

**Step 1: Write the failing test**

Add a test proving a protected route returns `401` without `x-loong-principal`.

**Step 2: Run test to verify it fails**

Run: `cargo test -p loong-memoryd missing_principal`
Expected: FAIL because protected-route principal extraction is not implemented.

**Step 3: Write minimal implementation**

- add shared request extraction/helper for `x-loong-principal`
- return a structured unauthorized error for missing or blank principals
- add request DTOs for put/get/recall/audit bodies

**Step 4: Run test to verify it passes**

Run: `cargo test -p loong-memoryd missing_principal`
Expected: PASS.

**Step 5: Commit**

```bash
git add crates/loong-memoryd/src/lib.rs crates/loong-memoryd/tests/http_service_integration.rs
git commit -m "feat: require principal header for service requests"
```

### Task 4: Implement put/get endpoints through `MemoryEngine`

**Files:**
- Modify: `crates/loong-memoryd/src/lib.rs`
- Modify: `crates/loong-memoryd/tests/http_service_integration.rs`

**Step 1: Write the failing test**

Add an end-to-end test:

- `POST /v1/memories` stores a record
- `POST /v1/memories/get` fetches it by `external_id`

**Step 2: Run test to verify it fails**

Run: `cargo test -p loong-memoryd put_and_get`
Expected: FAIL because handlers are not implemented.

**Step 3: Write minimal implementation**

- add a helper to run synchronous engine work inside `spawn_blocking`
- build a fresh engine per request from app state
- map request DTOs into `MemoryPutRequest` / `MemoryGetRequest`
- serialize `MemoryRecord` responses

**Step 4: Run test to verify it passes**

Run: `cargo test -p loong-memoryd put_and_get`
Expected: PASS.

**Step 5: Commit**

```bash
git add crates/loong-memoryd/src/lib.rs crates/loong-memoryd/tests/http_service_integration.rs
git commit -m "feat: add loong-memoryd put and get endpoints"
```

### Task 5: Implement recall and audit endpoints with policy-aware errors

**Files:**
- Modify: `crates/loong-memoryd/src/lib.rs`
- Modify: `crates/loong-memoryd/tests/http_service_integration.rs`

**Step 1: Write the failing test**

Add tests covering:

- recall returns a relevant hit for stored content
- audit returns namespace history after writes
- policy denial from a static policy file returns `403`

**Step 2: Run test to verify it fails**

Run: `cargo test -p loong-memoryd recall`
Run: `cargo test -p loong-memoryd audit`
Expected: FAIL because recall/audit handlers and error mapping are incomplete.

**Step 3: Write minimal implementation**

- implement `POST /v1/recall`
- implement `POST /v1/audit`
- normalize recall weights the same way the CLI does
- map `LoongMemoryError::PolicyDenied` to `403`
- keep count-bearing response envelopes for recall and audit

**Step 4: Run test to verify it passes**

Run: `cargo test -p loong-memoryd recall`
Run: `cargo test -p loong-memoryd audit`
Expected: PASS.

**Step 5: Commit**

```bash
git add crates/loong-memoryd/src/lib.rs crates/loong-memoryd/tests/http_service_integration.rs
git commit -m "feat: add loong-memoryd recall and audit endpoints"
```

### Task 6: Wire the binary runtime and update repository docs

**Files:**
- Modify: `crates/loong-memoryd/src/main.rs`
- Modify: `README.md`
- Modify: `docs/architecture.md`
- Modify: `docs/roadmap.md`

**Step 1: Write the failing test**

If practical, add a smoke test or command-level assertion that the binary config can be parsed and server state can be constructed from CLI inputs.

**Step 2: Run test to verify it fails**

Run: `cargo test -p loong-memoryd main`
Expected: FAIL until runtime wiring and config parsing are finalized.

**Step 3: Write minimal implementation**

- finalize CLI parsing for `--db`, `--listen-addr`, and `--policy-file`
- start the Axum server from a bound listener
- document service usage and curl examples in `README.md`
- update architecture and roadmap docs to describe the new daemon slice

**Step 4: Run test to verify it passes**

Run: `cargo test -p loong-memoryd main`
Expected: PASS.

**Step 5: Commit**

```bash
git add crates/loong-memoryd/src/main.rs README.md docs/architecture.md docs/roadmap.md
git commit -m "docs: document loong-memoryd service surface"
```

### Task 7: Run full workspace verification and prepare GitHub delivery

**Files:**
- Modify: `docs/research/phase2-service-evaluation-2026-03-15.md`

**Step 1: Write the failing test**

No new product test. This task is for verification and evidence capture.

**Step 2: Run test to verify it fails**

Not applicable.

**Step 3: Write minimal implementation**

- summarize design decisions, verification results, and residual risks in a new research note
- inspect `git status --short` and staged diff isolation

**Step 4: Run test to verify it passes**

Run:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Expected: all PASS.

**Step 5: Commit**

```bash
git add docs/research/phase2-service-evaluation-2026-03-15.md
git commit -m "feat: ship minimal loong-memoryd service surface"
```
