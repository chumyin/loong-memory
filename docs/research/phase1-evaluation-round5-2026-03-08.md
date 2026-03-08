# Phase 1 Deep Evaluation Round 5

Date: 2026-03-08
Scope: SQLite startup migration for legacy text vectors

## 1. Objective

Ensure older databases with JSON-text vectors are automatically upgraded to the
compact BLOB format on store startup, without breaking availability when legacy
rows are malformed.

## 2. Findings

After round3/round4, compatibility read-path was in place, but one gap remained:

- legacy JSON vectors were readable but not proactively compacted
- repeated runtime decode of legacy text vectors would continue indefinitely
  unless a migration path exists

## 3. Implementation

In `SqliteStore::open`:

- added schema migration marker check in `schema_migrations`
- added migration `v2`:
  - scans `memory_vectors` rows where `typeof(vector) = 'text'`
  - parses JSON vectors when valid
  - validates finiteness and non-empty vector payload
  - rewrites row as BLOB and synchronizes `dimension`
  - records `version = 2` migration marker atomically

Behavior for malformed legacy rows:

- malformed rows are skipped, migration still completes and marks v2
- recall path (from round4) remains resilient via row-level skip semantics

## 4. Verification

New integration test file:

- `sqlite_store_migration_integration.rs`

Covered scenarios:

- `open_migrates_legacy_text_vectors_to_blob_and_updates_dimension`
  - simulates pre-v2 DB marker
  - verifies startup migration rewrites text vectors to blob
  - verifies dimension is corrected
- `open_marks_migration_v2_and_recall_survives_invalid_legacy_text_vector`
  - simulates invalid legacy text vector row
  - verifies v2 marker is still recorded
  - verifies recall still succeeds for affected record

## 5. Quality Gates

Executed and passed:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`

Current test totals:

- `engine_store_integration`: 14 passed
- `sqlite_store_migration_integration`: 2 passed
- `audit_sqlite_integration`: 3 passed
- `policy_static_tests`: 2 passed
- tokenizer unit tests: 2 passed

## 6. Impact

Performance:

- startup compaction removes recurring JSON parse overhead for legacy vectors.

Reliability:

- migration is idempotent via schema version marker.

Compatibility:

- both valid and invalid legacy datasets are handled without service outage.

Maintainability:

- migration lifecycle is explicit and auditable in `schema_migrations`.
