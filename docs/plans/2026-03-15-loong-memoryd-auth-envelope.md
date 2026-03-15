# loong-memoryd Auth Envelope Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add an optional static bearer-token authentication mode to `loong-memoryd` while preserving trusted-header behavior for local operator use.

**Architecture:** Extend `ServiceConfig` and `ServiceState` with transport-auth configuration. Protected routes will authenticate using either the existing trusted-header mode or a new static-token mode that derives principal from `Authorization: Bearer <token>`, then continue routing authorization through the existing `MemoryEngine`.

**Tech Stack:** Rust, `axum`, `tokio`, `serde`, existing `loong-memory-core`

---

### Task 1: Add failing daemon integration tests for static bearer auth

**Files:**
- Modify: `crates/loong-memoryd/tests/http_service_integration.rs`

**Step 1: Write the failing test**

Add tests for:

- valid bearer token allows a protected request
- missing bearer token returns `401`
- invalid bearer token returns `401`
- spoofed `x-loong-principal` is ignored in token mode
- `/healthz` reports `auth_mode`

**Step 2: Run test to verify it fails**

Run: `cargo test -p loong-memoryd bearer`
Expected: FAIL because static-token auth mode does not exist yet.

**Step 3: Write minimal implementation**

Do not implement yet. This task establishes the red tests only.

**Step 4: Run test to verify it passes**

Not applicable in this task.

**Step 5: Commit**

```bash
git add crates/loong-memoryd/tests/http_service_integration.rs
git commit -m "test: add failing loong-memoryd auth envelope cases"
```

### Task 2: Add auth configuration types and trusted-header/static-token modes

**Files:**
- Modify: `crates/loong-memoryd/src/lib.rs`
- Modify: `crates/loong-memoryd/src/main.rs`

**Step 1: Write the failing test**

Use the new tests from Task 1 for:

- missing bearer token
- invalid bearer token
- `/healthz` auth mode reporting

**Step 2: Run test to verify it fails**

Run: `cargo test -p loong-memoryd missing_bearer`
Run: `cargo test -p loong-memoryd invalid_bearer`
Expected: FAIL because auth-file support and auth-mode selection do not exist.

**Step 3: Write minimal implementation**

- add `--auth-file` CLI support
- add auth config structs and loader
- add runtime `AuthMode`
- extend health response with `auth_mode`
- switch request authentication based on configured auth mode

**Step 4: Run test to verify it passes**

Run: `cargo test -p loong-memoryd healthz`
Run: `cargo test -p loong-memoryd bearer`
Expected: the mode-selection tests pass or progress substantially.

**Step 5: Commit**

```bash
git add crates/loong-memoryd/src/lib.rs crates/loong-memoryd/src/main.rs
git commit -m "feat: add loong-memoryd auth mode configuration"
```

### Task 3: Derive principal from bearer token and block spoofing

**Files:**
- Modify: `crates/loong-memoryd/src/lib.rs`
- Modify: `crates/loong-memoryd/tests/http_service_integration.rs`

**Step 1: Write the failing test**

Use the new tests from Task 1 for:

- valid bearer token request succeeds
- spoofed `x-loong-principal` does not bypass policy

**Step 2: Run test to verify it fails**

Run: `cargo test -p loong-memoryd bearer_token_allows`
Run: `cargo test -p loong-memoryd spoofed`
Expected: FAIL until bearer principal derivation is fully wired.

**Step 3: Write minimal implementation**

- parse `Authorization: Bearer <token>`
- look up token in the immutable token map
- derive principal from the token mapping
- ignore `x-loong-principal` in static-token mode

**Step 4: Run test to verify it passes**

Run: `cargo test -p loong-memoryd bearer`
Expected: PASS.

**Step 5: Commit**

```bash
git add crates/loong-memoryd/src/lib.rs crates/loong-memoryd/tests/http_service_integration.rs
git commit -m "feat: derive loong-memoryd principal from bearer auth"
```

### Task 4: Update docs and research notes

**Files:**
- Modify: `README.md`
- Modify: `docs/architecture.md`
- Modify: `docs/research/phase2-service-evaluation-2026-03-15.md`

**Step 1: Write the failing test**

No product test for this task.

**Step 2: Run test to verify it fails**

Not applicable.

**Step 3: Write minimal implementation**

- document `--auth-file`
- document static bearer-token mode and trusted-header fallback mode
- update the research note with the stronger auth envelope and auth-related verification

**Step 4: Run test to verify it passes**

Run: `cargo test -p loong-memoryd`
Expected: PASS with docs aligned to shipped behavior.

**Step 5: Commit**

```bash
git add README.md docs/architecture.md docs/research/phase2-service-evaluation-2026-03-15.md
git commit -m "docs: describe loong-memoryd auth envelope"
```

### Task 5: Run full verification and prepare GitHub delivery

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
git commit -m "feat: add loong-memoryd auth envelope"
```
