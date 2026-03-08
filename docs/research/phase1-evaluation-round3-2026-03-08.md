# Phase 1 Deep Evaluation Round 3

Date: 2026-03-08
Scope: `loong-memory-core` policy/store hardening + compatibility verification

## 1. Objective

Round 3 focused on two kernel-level improvements that affect security and
forward compatibility:

- policy expressiveness at principal granularity
- compact vector persistence with backward compatibility

## 2. Findings and Implementation

## 2.1 Principal-Scoped Authorization in `StaticPolicy`

Finding:

- Existing static policy only expressed namespace-wide action permissions.
- That model is insufficient for principal-level least privilege.

Implementation:

- Extended `StaticPolicy` with two allow maps:
  - namespace-wide allow map
  - `(principal, namespace)` scoped allow map
- Added builder:
  - `allow_principal_namespace_actions(principal, namespace, actions)`
- Decision order:
  1. check principal+namespace allow-list
  2. fallback to namespace-wide allow-list
  3. deny by default

Verification:

- new test file `policy_static_tests.rs` validates:
  - principal-scoped allow behavior
  - deny-by-default semantics

## 2.2 Vector Persistence Migration: JSON Text -> BLOB

Finding:

- Vector rows were stored as JSON text, causing avoidable storage and parse
  overhead.
- Existing deployments may still have JSON-text vector rows.

Implementation:

- Migrated `memory_vectors.vector` write path to compact `f32` LE BLOB encoding.
- Added decode compatibility path:
  - preferred decode from SQLite `BLOB`
  - fallback decode from legacy JSON `TEXT`
- Added explicit helper functions:
  - `encode_vector_blob`
  - `decode_vector_blob`
  - `decode_vector_value`

Verification:

- integration test `vector_storage_uses_blob_and_reads_legacy_json_vectors`:
  - asserts new writes are SQLite `blob`
  - forcibly overwrites vector row with legacy JSON text
  - confirms recall still succeeds

## 3. Quality Gates (Round 3)

Executed and passed:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`

Relevant test signals:

- `engine_store_integration`: 12 passed
- `audit_sqlite_integration`: 3 passed
- `policy_static_tests`: 2 passed
- tokenizer unit tests: 2 passed

## 4. Impact Assessment

Security:

- improved least-privilege policy expression by principal within namespace.

Performance:

- lower vector storage footprint and decode overhead via binary BLOB format.

Compatibility:

- legacy JSON vector rows remain readable without data migration freeze.

Maintainability:

- clearer policy contracts and vector decode boundary with dedicated tests.

## 5. Residual Risk and Next Step

- recall vector similarity still uses bounded local scan; large datasets need
  ANN/vector indexing strategy in Phase 2.
- static policy is still config-based; dynamic policy engines (ABAC/RBAC/time)
  remain next-phase work.
