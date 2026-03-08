# Phase 1 Deep Evaluation Round 4

Date: 2026-03-08
Scope: vector data integrity + recall-path resilience

## 1. Objective

This round focused on making recall robust under partial data corruption and
embedding drift scenarios, without breaking backwards compatibility.

## 2. Findings

Pre-change behavior risk:

- vector candidate decode errors (for example malformed blob payload) could fail
  the entire recall request.
- non-finite values (NaN/Inf) in stored vectors could contaminate cosine scoring.
- dimension mismatches between stored vectors and query embedding dimension were
  not explicitly filtered at candidate stage.

## 3. Implementation

## 3.1 Candidate Integrity Checks

In `SqliteStore::read_vector_candidates`:

- fetches `memory_id`, `dimension`, and vector payload.
- rejects rows where stored dimension does not match query vector length.
- decodes vector payload with strict checks:
  - blob/text decode
  - decoded length equals declared dimension
  - all values finite
- skips invalid rows instead of failing the whole recall call.

This converts row-level corruption from a system failure into degraded scoring
for affected rows.

## 3.2 Decode-Path Validation

`decode_vector_value` now enforces:

- expected dimension equality
- finite numeric values only

Malformed rows now surface as decode errors at row scope and are skipped by
candidate reader.

## 4. Verification

Added integration tests:

- `recall_skips_corrupted_vector_blob_instead_of_failing`
- `recall_skips_non_finite_vector_values`

Behavior validated:

- recall succeeds when vector payload is malformed or non-finite
- affected row can still be returned via lexical channel
- `vector_score` degrades to `0.0` for invalid vector rows

## 5. Quality Gates

Executed and passed:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`

Current test totals:

- `engine_store_integration`: 14 passed
- `audit_sqlite_integration`: 3 passed
- `policy_static_tests`: 2 passed
- tokenizer unit tests: 2 passed

## 6. Impact

Reliability:

- single-row vector corruption no longer takes down whole recall path.

Safety:

- non-finite vectors are explicitly excluded from scoring.

Correctness:

- vector channel requires dimension-consistent candidates.

Compatibility:

- legacy JSON vector decoding remains supported (from prior round), while
  integrity checks apply uniformly.

## 7. Residual Risk

- current fallback strategy silently skips bad rows; a future health-check
  command can expose skipped-row diagnostics for operator visibility.
- vector retrieval is still local bounded scan; large-scale recall still needs
  ANN/index acceleration in Phase 2.
