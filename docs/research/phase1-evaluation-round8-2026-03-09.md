# Phase 1 Deep Evaluation Round 8

Date: 2026-03-09
Scope: maintenance operation security hardening (policy + audit control plane)

## 1. Objective

Round7 introduced vector repair workflows. A security review found an important
control-plane gap:

- CLI maintenance commands (`vector-health`, `vector-repair`) directly invoked
  `SqliteStore` APIs, bypassing engine-level policy gates and standardized audit
  emission.

This round closes that gap and unifies maintenance operations under the same
authorization/audit boundaries as core CRUD/recall operations.

## 2. Findings

Pre-change risk:

- privileged maintenance capabilities were reachable without policy enforcement
  semantics in the CLI path.
- audit events for maintenance operations were not guaranteed through engine.

## 3. Implementation

## 3.1 Policy Surface Extension

- added `Action::Repair` in policy action model.

Rationale:

- distinguishes repair writes from normal put/delete semantics.
- allows fine-grained principal+namespace grants in static policy configs.

## 3.2 Store Contract Extension

`MemoryStore` trait now defines maintenance hooks:

- `vector_health_report(namespace, invalid_sample_limit)`
- `vector_repair(namespace, issue_sample_limit, apply)`

Default behavior is explicit `NotImplemented` for non-supporting backends.

## 3.3 Engine-Level Maintenance Operations

Added to `MemoryEngine`:

- `vector_health(ctx, namespace, invalid_sample_limit)`
  - enforces `Action::AuditRead`
  - emits read audit event with row summary details
- `vector_repair(ctx, namespace, issue_sample_limit, apply)`
  - enforces `Action::Repair`
  - emits read/write maintenance audit event based on `apply`

## 3.4 CLI Path Hardening

`vector-health` and `vector-repair` now:

- require `--namespace`
- accept `--principal`
- route through `MemoryEngine` methods instead of direct store calls

This removes policy bypass behavior from maintenance command paths.

## 4. Verification

New/updated tests:

- `engine_store_integration::vector_health_and_repair_are_policy_gated`
  - confirms health allowed with `AuditRead`
  - confirms repair denied without `Action::Repair`
  - confirms corresponding audit events are emitted
- existing repair/health/store integration suites remain green

CLI verification:

- `loong-memory vector-repair --help` confirms required namespace + principal
- command routing remains functional through engine

## 5. Quality Gates

Executed and passed:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`

Current test totals:

- `engine_store_integration`: 15 passed
- `sqlite_store_migration_integration`: 2 passed
- `vector_health_integration`: 2 passed
- `vector_repair_integration`: 2 passed
- `audit_sqlite_integration`: 3 passed
- `policy_static_tests`: 2 passed
- tokenizer unit tests: 2 passed

## 6. Impact

Security:

- closes maintenance-operation authorization bypass path.

Compliance/Auditability:

- maintenance operations now inherit standardized engine audit flows.

Maintainability:

- backend-agnostic maintenance contract is explicit in store trait.

## 7. Residual Risk

- policy remains static allow-list model; dynamic RBAC/ABAC policy engines are
  still next-phase work.
- maintenance actions are auditable at operation summary level; row-level repair
  audit expansion can be added in future rounds.
