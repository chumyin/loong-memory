# Roadmap

## Phase 1 (Completed): Engine-first + CLI

- core engine contracts and policy/audit boundaries
- SQLite store with FTS + vector hybrid retrieval
- CLI for operational CRUD and audit inspection
- baseline integration tests and strict lint gates

## Phase 2: Transport + SDK Surface

- standalone daemon process (`loong-memoryd`)
- gRPC/HTTP APIs with authn/authz envelope
- Rust + TypeScript SDK clients
- metrics/tracing integration and structured health checks

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
