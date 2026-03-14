# Audit Guarantees and CLI Control Plane Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Harden audit persistence semantics and make `loong-memory-cli` a real policy-aware operator surface by adding static policy-file loading and moving audit reads behind engine-level authorization and audit boundaries.

**Architecture:** Make the audit contract fallible and append-only, preserve namespace-scoped audit reads through `MemoryEngine`, then wire the CLI to load a JSON `StaticPolicy` and route `audit` through the same governed path as other protected operations. Keep backward compatibility by defaulting to `AllowAllPolicy` when no policy file is supplied.

**Tech Stack:** Rust workspace, `clap`, `serde`, `serde_json`, `rusqlite`, `tempfile`, `std::process::Command`

---

### Task 1: Add optional read-side audit support in core

**Files:**
- Modify: `crates/loong-memory-core/src/audit.rs`
- Modify: `crates/loong-memory-core/src/lib.rs`

**Step 1: Write the failing test**

Add/prepare a core integration test that tries to call engine audit-read without a supporting audit sink and expects `NotImplemented`.

**Step 2: Run test to verify it fails**

Run: `cargo test -p loong-memory-core audit_read_without_reader_is_not_implemented -- --exact`
Expected: FAIL because the engine method/contract does not exist yet.

**Step 3: Write minimal implementation**

- extend `AuditSink` with namespace-scoped `list(...)`
- keep default behavior as `NotImplemented`
- implement list support for `SqliteAuditSink`
- export the updated contract from `lib.rs`

**Step 4: Run test to verify it passes**

Run: `cargo test -p loong-memory-core audit_read_without_reader_is_not_implemented -- --exact`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/loong-memory-core/src/audit.rs crates/loong-memory-core/src/lib.rs
git commit -m "feat(core): add audit log reader contract"
```

### Task 2: Refactor engine authorization flow and add audit-read operation

**Files:**
- Modify: `crates/loong-memory-core/src/engine.rs`
- Modify: `crates/loong-memory-core/tests/engine_store_integration.rs`

**Step 1: Write the failing test**

Add focused tests for:

- audit read denied without `Action::AuditRead`
- audit read allowed with `Action::AuditRead`
- audit read result excludes self-generated audit-read events

**Step 2: Run test to verify it fails**

Run: `cargo test -p loong-memory-core audit_read -- --nocapture`
Expected: FAIL because engine audit-read does not exist or behavior is wrong.

**Step 3: Write minimal implementation**

- split authorization decision from allow-event emission
- keep deny events immediate
- add engine audit-read method using the attached audit sink
- emit allow/read events after collecting the history

**Step 4: Run test to verify it passes**

Run: `cargo test -p loong-memory-core audit_read -- --nocapture`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/loong-memory-core/src/engine.rs crates/loong-memory-core/tests/engine_store_integration.rs
git commit -m "feat(core): gate audit reads through memory engine"
```

### Task 3: Add JSON static policy config support

**Files:**
- Modify: `crates/loong-memory-core/src/policy.rs`
- Modify: `crates/loong-memory-core/src/lib.rs`
- Test: `crates/loong-memory-core/tests/policy_static_tests.rs`

**Step 1: Write the failing test**

Add tests that deserialize JSON policy config with:

- namespace-level actions
- principal+namespace actions
- `audit_read` and `repair` action names

**Step 2: Run test to verify it fails**

Run: `cargo test -p loong-memory-core static_policy_ -- --nocapture`
Expected: FAIL because serde config support is missing.

**Step 3: Write minimal implementation**

- derive serde support for `Action`
- define a config shape that maps directly to `StaticPolicy`
- add constructor/helper that builds `StaticPolicy` from config

**Step 4: Run test to verify it passes**

Run: `cargo test -p loong-memory-core static_policy_ -- --nocapture`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/loong-memory-core/src/policy.rs crates/loong-memory-core/src/lib.rs crates/loong-memory-core/tests/policy_static_tests.rs
git commit -m "feat(policy): support static policy config files"
```

### Task 4: Wire CLI to load policy files and route audit through engine

**Files:**
- Modify: `crates/loong-memory-cli/src/main.rs`

**Step 1: Write the failing test**

Create CLI integration tests covering:

- `audit` requires `--namespace`
- `audit` requires `--principal`
- `--policy-file` allows `put` and `audit` for an operator principal
- `--policy-file` denies `audit` when `audit_read` is missing

**Step 2: Run test to verify it fails**

Run: `cargo test -p loong-memory-cli --test cli_policy_integration -- --nocapture`
Expected: FAIL because the CLI has no policy-file support and `audit` still bypasses engine.

**Step 3: Write minimal implementation**

- add a global `--policy-file` CLI flag
- load JSON policy file into `StaticPolicy`
- require `namespace` and `principal` on `audit`
- route `audit` through the new engine method

**Step 4: Run test to verify it passes**

Run: `cargo test -p loong-memory-cli --test cli_policy_integration -- --nocapture`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/loong-memory-cli/src/main.rs crates/loong-memory-cli/tests/cli_policy_integration.rs
git commit -m "feat(cli): add policy-aware audit control plane"
```

### Task 5: Publish docs and operator examples

**Files:**
- Modify: `README.md`
- Modify: `docs/architecture.md`
- Create: `docs/examples/static-policy.example.json`
- Create: `docs/research/phase1-evaluation-round9-2026-03-14.md`

**Step 1: Write the failing test**

Use command-output verification instead of unit tests:

- `cargo run -p loong-memory-cli -- audit --help`
- `cargo run -p loong-memory-cli -- --help`

Check that help text and README examples are now aligned.

**Step 2: Run command to verify old behavior**

Run: `cargo run -p loong-memory-cli -- audit --help`
Expected: output still shows optional namespace / no policy-file guidance before the docs update.

**Step 3: Write minimal implementation**

- update quick-start and security sections
- add operator-facing example policy file
- record the round9 rationale and verification evidence

**Step 4: Run command to verify it passes**

Run: `cargo run -p loong-memory-cli -- audit --help`
Expected: help matches required namespace/principal semantics and README docs.

**Step 5: Commit**

```bash
git add README.md docs/architecture.md docs/examples/static-policy.example.json docs/research/phase1-evaluation-round9-2026-03-14.md
git commit -m "docs: document cli policy control plane"
```

### Task 6: Run full workspace verification

**Files:**
- Modify only if verification reveals issues

**Step 1: Run formatting**

Run: `cargo fmt --all -- --check`
Expected: PASS

**Step 2: Run lint**

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: PASS

**Step 3: Run tests**

Run: `cargo test --workspace`
Expected: PASS

**Step 4: Inspect git isolation**

Run:

```bash
git status --short
git diff --cached --name-only
git diff --cached
```

Expected: only task-scoped changes are present.

**Step 5: Commit**

```bash
git add README.md docs crates
git commit -m "feat: harden cli policy control plane"
```
