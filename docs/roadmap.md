# Roadmap

## Phase 1 (Completed): Engine-first + CLI

- core engine contracts and policy/audit boundaries
- SQLite store with FTS + vector hybrid retrieval
- CLI for operational CRUD and audit inspection
- baseline integration tests and strict lint gates

## Phase 2 (Started): Transport + SDK Surface

Implemented:

- standalone daemon process bootstrap (`loong-memoryd`)
- minimal HTTP JSON API with principal header envelope
- structured health checks

Remaining:

- broader HTTP/gRPC surface
- Rust + TypeScript SDK clients
- metrics/tracing integration

## Phase 3: Scalability and Isolation Hardening

- pluggable distributed backends (e.g., Postgres/pgvector)
- snapshot/backup primitives and migration tooling
- tenant-level quotas and policy packs
- per-namespace encryption/key-management integration

## Phase 4: Advanced Retrieval and Governance

- multi-vector and rerank pipelines
- semantic cache and recency-aware ranking
- retention/expiry policies and legal hold controls
- compliance-oriented audit export channels
