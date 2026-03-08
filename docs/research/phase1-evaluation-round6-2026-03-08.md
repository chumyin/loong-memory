# Phase 1 Deep Evaluation Round 6

Date: 2026-03-08
Scope: vector integrity observability (core diagnostics + CLI surfacing)

## 1. Objective

Round4 introduced resilient recall by skipping invalid vector rows. The residual
risk was observability: operators had no direct way to inspect how many rows
were skipped or why.

This round adds explicit diagnostics so resilience is measurable, not silent.

## 2. Findings

Before this round:

- invalid vector rows were skipped safely in recall
- there was no first-class API/CLI to quantify invalid rows
- troubleshooting required ad-hoc SQL inspection

## 3. Implementation

## 3.1 Core Diagnostics API

In `SqliteStore`:

- added `vector_health_report(namespace, invalid_sample_limit)`
- added serializable reports:
  - `VectorHealthReport`
  - `VectorHealthIssue`

Report dimensions:

- total row count
- blob/text row count
- valid/invalid row count
- sampled invalid rows (`memory_id`, SQLite type, reason)

Validation logic reuses vector decode checks:

- dimension sanity
- decode success (blob/text)
- finite numeric values

## 3.2 CLI Command

Added command:

- `loong-memory vector-health --db <path> [--namespace <ns>] [--invalid-sample-limit <n>]`

This makes vector integrity diagnostics operationally accessible without custom
SQL scripts.

## 4. Verification

New integration test file:

- `vector_health_integration.rs`

Covered scenarios:

- invalid blob + NaN vector rows are counted as invalid
- namespace filter behavior is correct
- text vector rows are counted and invalid sample limit is enforced

## 5. Quality Gates

Executed and passed:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`

Current test totals:

- `engine_store_integration`: 14 passed
- `sqlite_store_migration_integration`: 2 passed
- `vector_health_integration`: 2 passed
- `audit_sqlite_integration`: 3 passed
- `policy_static_tests`: 2 passed
- tokenizer unit tests: 2 passed

## 6. Impact

Reliability:

- row-level resilience remains intact from round4.

Observability:

- invalid vector data becomes measurable and inspectable.

Operations:

- CLI-level health checks enable routine integrity audits in maintenance flows.

## 7. Residual Risk

- diagnostics are read-only; auto-repair/remediation tooling is still future work.
- large-scale deployments still need ANN/index acceleration for recall latency.
