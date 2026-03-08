# Loong Memory Design (Phase 1 Engine-First)

Date: 2026-03-08
Owner: chumyin

## 1. Objectives

`loong-memory` is a Rust-native memory engine for agent systems. Phase 1 provides:

- deterministic core API for memory lifecycle
- security-first policy gates and audit events
- local high-performance storage with lexical + vector hybrid retrieval
- CLI operations for initialization, write, read, recall, and delete

## 2. Non-Goals (Phase 1)

- distributed replication/sharding
- external network service runtime (daemon/gRPC/HTTP)
- online model hosting inside this repository

## 3. Layered Architecture

### L0 Contract Layer

Stable types and errors:

- memory record and metadata contract
- retrieval request/response contract
- policy action contract
- audit event contract

### L1 Security & Policy Layer

- namespace-scoped action authorization
- fail-closed deny by default
- input size limits and metadata guardrails
- explicit principal tracking

### L2 Storage & Index Layer

SQLite backend with:

- normalized memory rows
- f32 vector blobs
- FTS5 lexical index
- deterministic schema migration table

### L3 Retrieval Layer

Hybrid strategy:

- lexical candidate set via FTS5
- vector candidate set via cosine similarity
- weighted merge with stable tie-break rules

### L4 Orchestration Layer

`MemoryEngine` composes policy + store + embedder + audit.

## 4. Data Model

Core entity: `MemoryRecord`

- `id: UUID`
- `namespace: String`
- `external_id: Option<String>`
- `content: String`
- `metadata: serde_json::Value` (object)
- `content_hash: sha256`
- `created_at`, `updated_at`

Retrieval entity: `RecallHit`

- memory record
- lexical score
- vector score
- hybrid score

## 5. Security Model

- Policy deny is default.
- Every mutating operation requires explicit action allow.
- Namespace is mandatory for all calls.
- Max content bytes and metadata bytes are configurable.
- Metadata must be JSON object to avoid polymorphic confusion.
- Audit records include principal, action, namespace, and outcome.

## 6. Performance Model

- SQLite WAL mode and bounded busy timeout.
- Explicit indexes on `(namespace, updated_at)` and `(namespace, external_id)`.
- FTS5 for lexical retrieval.
- Vector similarity scan is bounded by `vector_scan_limit`.
- Upsert path uses transactions and minimizes re-index writes.

## 7. Quality Strategy

- test-first for policy, store, retrieval, and audit behavior
- deterministic embedding provider for reproducible tests
- integration tests use temp SQLite databases
- strict linting (`clippy -D warnings`) and formatting

## 8. Extensibility Contracts

Traits:

- `EmbeddingProvider`
- `MemoryStore`
- `PolicyEngine`
- `AuditSink`

This allows later replacement of:

- embedding provider (cloud or local model)
- storage backend (Postgres/vector DB)
- policy plugin chain
- SIEM-compatible audit sink

## 9. Phase Plan

### Phase 1 (current)

- core engine + sqlite store + CLI

### Phase 2

- daemon transport and SDK binding

### Phase 3

- distributed replication and consistency model

## 10. Risk Controls

- schema migration version checks with fail-fast behavior
- strict parse/validation on user input
- audit on both allow and deny path
- deterministic score merge to avoid flaky ordering
