# Phase 1 Deep Evaluation Round 7

Date: 2026-03-08
Scope: vector repair workflow (plan + apply)

## 1. Objective

Round6 added vector integrity observability. The remaining operational gap was
remediation: operators could see invalid/repairable rows but could not execute a
safe, structured repair flow from the core API/CLI.

This round adds a controlled repair pipeline with explicit dry-run and apply
modes.

## 2. Findings

Prior state:

- vector health diagnostics could detect bad rows
- no first-class API to repair recoverable rows
- remediation required manual SQL edits (high risk, low repeatability)

## 3. Implementation

## 3.1 Core Repair API

In `SqliteStore`:

- added `vector_repair(namespace, issue_sample_limit, apply)`
- added serializable models:
  - `VectorRepairReport`
  - `VectorRepairIssue`

Repair logic:

- scans candidate rows in scope (`namespace` or global)
- classifies rows into:
  - `repairable`: valid payload but requires canonicalization/fix
  - `invalid`: decode/dimension/finiteness failures
  - `unchanged`: already healthy/canonical
- repairable rules:
  - text JSON vector -> canonical BLOB rewrite + dimension sync
  - blob vector with dimension mismatch -> dimension + BLOB rewrite from decoded payload
- apply mode:
  - executes transactional update for repairable rows
- dry-run mode:
  - reports plan only, no writes

## 3.2 CLI Repair Command

Added command:

- `loong-memory vector-repair --db <path> [--namespace <ns>] [--issue-sample-limit <n>] [--apply]`

Defaults:

- dry-run by default (`--apply` required to persist changes)

## 4. Verification

New integration test file:

- `vector_repair_integration.rs`

Covered scenarios:

- `vector_repair_dry_run_reports_changes_without_writing`
  - verifies repair planning and no write side effects
- `vector_repair_apply_converts_text_and_fixes_dimension_mismatch`
  - verifies transactional repair of recoverable rows
  - verifies invalid rows remain flagged/reported

CLI surface check:

- `loong-memory --help` includes `vector-repair`

## 5. Quality Gates

Executed and passed:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`

Current test totals:

- `engine_store_integration`: 14 passed
- `sqlite_store_migration_integration`: 2 passed
- `vector_health_integration`: 2 passed
- `vector_repair_integration`: 2 passed
- `audit_sqlite_integration`: 3 passed
- `policy_static_tests`: 2 passed
- tokenizer unit tests: 2 passed

## 6. Impact

Reliability:

- converts remediation from manual SQL to structured API flow.

Safety:

- default dry-run lowers accidental write risk.

Operations:

- detect -> plan -> apply is now fully supported via CLI and core API.

## 7. Residual Risk

- invalid rows still need operator review when payload is irrecoverable.
- future enhancement can add selective repair filtering by issue type and audit
  export of repair actions.
