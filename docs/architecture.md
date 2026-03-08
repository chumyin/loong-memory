# loong-memory Architecture (Phase 1)

## 1. Design Goals

- Predictable behavior under high-frequency agent reads/writes.
- Strict namespace isolation across tenants/agents.
- Swap-friendly contracts for model provider, policy engine, and audit sink.
- Deterministic retrieval behavior for reproducible debugging.

## 2. Layered Runtime

### L0 Contract Layer

Core contracts in `loong-memory-core`:

- `MemoryStore`: persistence and retrieval backend.
- `EmbeddingProvider`: text to vector interface.
- `PolicyEngine`: authorization decision boundary.
- `AuditSink`: immutable operation trail sink.

### L1 Control Layer (`MemoryEngine`)

`MemoryEngine` orchestrates all operations:

1. Validate request payload and bounds.
2. Enforce policy (`Allow`/`Deny`).
3. Execute store call.
4. Emit audit events for allow/deny and operation result.

This keeps control-plane decisions outside storage internals.

Validation guardrails at this layer include:

- namespace / external-id size limits
- content / metadata / query byte-size limits
- selector consistency (`id` XOR `external_id`)
- recall weight sanity (finite, non-negative, positive-sum)

### L2 Store Layer (`SqliteStore`)

SQLite schema includes:

- `memories`: canonical memory rows.
- `memory_vectors`: compact `f32` little-endian BLOB by memory ID
  (legacy JSON-text vector rows are still read-compatible).
- `memory_fts`: FTS5 lexical index.
- `memory_audit`: durable audit events.

Store behavior:

- Upsert by `(namespace, external_id)` when external ID is provided.
- Strict selector semantics for get/delete (`id` XOR `external_id`).
- Transactional update of row + vector + FTS index.
- Schema migration `v2` auto-converts legacy JSON-text vectors to compact BLOB
  on `SqliteStore::open` (invalid legacy rows are skipped safely).
- `vector_health_report(namespace, sample_limit)` provides diagnostic counts and
  sampled invalid-row reasons for operator observability.
- `vector_repair(namespace, sample_limit, apply)` provides repair planning and
  optional transactional apply for recoverable rows.

### L3 Retrieval Layer (Hybrid Recall)

For each query:

- Lexical candidate set via FTS5 `MATCH` + `bm25` ranking.
- If FTS lexical candidates are empty, fallback lexical scoring uses bounded token-overlap
  over recent namespace rows (improves CJK/non-space language handling).
- Vector candidate set via bounded scan + cosine similarity.
- Vector candidates enforce dimension and finiteness checks; malformed rows are skipped
  so single-row corruption does not fail entire recall.
- Candidate union + weighted merge:
  - `hybrid = lexical_weight * lexical_score + vector_weight * vector_score`
- Deterministic sort:
  - `hybrid_score DESC`, `updated_at DESC`, `id ASC`.

## 3. Data Model

### MemoryRecord

- `id`: UUID string.
- `namespace`: tenant/agent isolation key.
- `external_id`: optional business key scoped by namespace.
- `content`: source memory text.
- `metadata`: JSON object payload.
- `content_hash`: SHA-256 digest.
- `created_at`, `updated_at`: RFC3339 UTC timestamps.

### RecallHit

- `record`: matched memory.
- `lexical_score`: normalized lexical score `[0, 1]`.
- `vector_score`: normalized cosine score `[0, 1]`.
- `hybrid_score`: weighted final score.

## 4. Security Model

- Policy gate is enforced before every operation.
- Maintenance operations (`vector_health`, `vector_repair`) are executed through
  engine-level policy checks and audit events, not direct CLI store bypass.
- Namespace is mandatory at contract level and store query level.
- Selector ambiguity (`id` + `external_id`) is rejected.
- Audit captures deny and allow paths for traceability.

## 5. Performance Model

- SQLite WAL mode for concurrent read/write workloads.
- `busy_timeout` to absorb transient write contention.
- FTS and vector candidate scans are bounded to avoid unbounded CPU.
- recall request limit is capped at engine level (`max_recall_limit`).
- Indexes:
  - `(namespace, updated_at DESC)`
  - `(namespace, external_id)` unique partial index

## 6. Extensibility Strategy

Future swaps without changing API consumers:

- Replace `DeterministicHashEmbedder` with remote/local model adapter.
- Replace `SqliteStore` with pgvector or distributed memory backend.
- Compose richer policy engines (RBAC/ABAC/time-based constraints).
- Ship multiple audit sinks (SQLite, Kafka, SIEM webhook).

## 7. Quality and Test Strategy

Current integration tests validate:

- policy deny path + audit deny emission
- namespace isolation
- put/get/delete roundtrip and upsert semantics
- hybrid recall ranking behavior
- allow + operation audit coverage
- selector and weight validation edge cases
- recall upper-bound protection and multilingual CJK retrieval behavior
- vector BLOB persistence plus legacy JSON-text vector read compatibility
- corrupted/non-finite vector row resilience in recall path
- startup migration behavior for legacy text vectors (v2 marker path)
- vector health diagnostics over valid/invalid blob/text rows
- vector repair workflow (`dry-run` planning + transactional `apply`)
- principal+namespace scoped static policy behavior
- audit SQLite persistence/filter/limit/error behavior

Quality gates:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
