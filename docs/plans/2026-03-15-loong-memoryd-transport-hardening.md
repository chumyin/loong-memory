# loong-memoryd Transport Hardening Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Broaden `loong-memoryd` so it exposes delete and maintenance operations over HTTP and pins service-level negative-path behavior with end-to-end tests.

**Architecture:** Extend the existing Axum router and per-request `MemoryEngine` execution model. Reuse current engine request/response types where possible, add small transport DTOs for maintenance routes, and harden the HTTP layer with integration tests for malformed JSON, validation failures, and readiness failure.

**Tech Stack:** Rust, `axum`, `tokio`, `reqwest`, existing `loong-memory-core`

---

### Task 1: Add failing HTTP integration tests for the new transport slice

**Files:**
- Modify: `crates/loong-memoryd/tests/http_service_integration.rs`

**Step 1: Write the failing test**

Add tests for:

- delete over HTTP
- vector health over HTTP
- vector repair dry-run over HTTP
- malformed JSON returns `invalid_json`
- invalid recall weights return `validation_failed`
- invalid delete selector returns `validation_failed`
- `/healthz` readiness failure returns `500`

**Step 2: Run test to verify it fails**

Run: `cargo test -p loong-memoryd`
Expected: FAIL because these routes and behaviors are not all implemented yet.

**Step 3: Write minimal implementation**

Do not implement yet. This task ends with failing tests in place.

**Step 4: Run test to verify it passes**

Not applicable in this task.

**Step 5: Commit**

```bash
git add crates/loong-memoryd/tests/http_service_integration.rs
git commit -m "test: add failing loong-memoryd transport hardening cases"
```

### Task 2: Implement delete transport support

**Files:**
- Modify: `crates/loong-memoryd/src/lib.rs`
- Modify: `crates/loong-memoryd/tests/http_service_integration.rs`

**Step 1: Write the failing test**

Use the new tests from Task 1 for:

- delete success
- invalid delete selector

**Step 2: Run test to verify it fails**

Run: `cargo test -p loong-memoryd delete`
Expected: FAIL because `DELETE /v1/memories` is not implemented.

**Step 3: Write minimal implementation**

- register `DELETE /v1/memories`
- accept `MemoryDeleteRequest` as JSON body
- route through `MemoryEngine::delete`
- return `{ "ok": true }` on success

**Step 4: Run test to verify it passes**

Run: `cargo test -p loong-memoryd delete`
Expected: PASS.

**Step 5: Commit**

```bash
git add crates/loong-memoryd/src/lib.rs crates/loong-memoryd/tests/http_service_integration.rs
git commit -m "feat: add loong-memoryd delete endpoint"
```

### Task 3: Implement vector health and vector repair HTTP routes

**Files:**
- Modify: `crates/loong-memoryd/src/lib.rs`
- Modify: `crates/loong-memoryd/tests/http_service_integration.rs`

**Step 1: Write the failing test**

Use the new tests from Task 1 for:

- vector health success
- vector repair dry-run success
- maintenance-route policy denial

**Step 2: Run test to verify it fails**

Run: `cargo test -p loong-memoryd vector`
Expected: FAIL because the maintenance routes are not implemented.

**Step 3: Write minimal implementation**

- add request DTOs for maintenance routes
- register `POST /v1/vector-health`
- register `POST /v1/vector-repair`
- route through the engine methods without bypassing policy or audit logic

**Step 4: Run test to verify it passes**

Run: `cargo test -p loong-memoryd vector`
Expected: PASS.

**Step 5: Commit**

```bash
git add crates/loong-memoryd/src/lib.rs crates/loong-memoryd/tests/http_service_integration.rs
git commit -m "feat: add loong-memoryd maintenance endpoints"
```

### Task 4: Harden negative-path transport behavior

**Files:**
- Modify: `crates/loong-memoryd/src/lib.rs`
- Modify: `crates/loong-memoryd/tests/http_service_integration.rs`

**Step 1: Write the failing test**

Use the new tests from Task 1 for:

- malformed JSON
- invalid recall weights
- readiness failure on `/healthz`

**Step 2: Run test to verify it fails**

Run: `cargo test -p loong-memoryd invalid_json`
Run: `cargo test -p loong-memoryd healthz`
Expected: FAIL until transport behavior is fully pinned.

**Step 3: Write minimal implementation**

- add any missing helpers required for testability
- ensure readiness failure uses the structured error envelope
- preserve existing `invalid_json` and validation error mapping

**Step 4: Run test to verify it passes**

Run: `cargo test -p loong-memoryd`
Expected: PASS.

**Step 5: Commit**

```bash
git add crates/loong-memoryd/src/lib.rs crates/loong-memoryd/tests/http_service_integration.rs
git commit -m "test: harden loong-memoryd negative-path coverage"
```

### Task 5: Update docs and research notes

**Files:**
- Modify: `README.md`
- Modify: `docs/architecture.md`
- Modify: `docs/research/phase2-service-evaluation-2026-03-15.md`

**Step 1: Write the failing test**

No product test. This task updates documentation and evidence.

**Step 2: Run test to verify it fails**

Not applicable.

**Step 3: Write minimal implementation**

- document the expanded route set
- add example delete and maintenance requests
- update the research note with the broadened service surface and new negative-path verification

**Step 4: Run test to verify it passes**

Run: `cargo test -p loong-memoryd`
Expected: PASS with docs updated alongside the implementation.

**Step 5: Commit**

```bash
git add README.md docs/architecture.md docs/research/phase2-service-evaluation-2026-03-15.md
git commit -m "docs: update loong-memoryd transport coverage"
```

### Task 6: Run full verification and prepare GitHub delivery

**Files:**
- No additional product files required

**Step 1: Write the failing test**

No new test.

**Step 2: Run test to verify it fails**

Not applicable.

**Step 3: Write minimal implementation**

- inspect `git status --short`
- inspect `git diff --cached --name-only`
- inspect `git diff --cached`

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
git commit -m "feat: harden loong-memoryd transport surface"
```
