# loong-memory

`loong-memory` is a standalone Rust memory engine for agent systems.
It is designed around four non-negotiable goals:

- Extensibility: trait-based contracts for store/embedder/policy/audit.
- Performance: SQLite WAL, FTS5 lexical search, and bounded vector recall.
- Quality: deterministic test strategy and strict lint/format gates.
- Security: namespace isolation, policy gate before data access, auditable operations.

## Current Status

Phase 1 (Engine-first + CLI) is implemented:

- `loong-memory-core`
  - transactional SQLite memory store
  - deterministic hash embedding provider with multilingual tokenization
  - hybrid recall (FTS + cosine similarity)
  - policy enforcement (namespace + principal-scoped static rules)
  - vector persistence as compact BLOB with legacy JSON read compatibility
  - audit sink contracts
- `loong-memory-cli`
  - `init`, `put`, `get`, `recall`, `delete`, `audit`

## Repository Layout

- `crates/loong-memory-core`: core contracts and engine implementation.
- `crates/loong-memory-cli`: command-line operational entrypoint.
- `docs/architecture.md`: architecture and data model details.
- `docs/research/onecontext-reverse-engineering.md`: onecontext implementation analysis and extracted design lessons.
- `docs/research/phase1-evaluation-2026-03-08.md`: deep evaluation, optimization decisions, and verification results.
- `docs/research/phase1-evaluation-round2-2026-03-08.md`: multilingual retrieval hardening and recall-bound enforcement.
- `docs/research/phase1-evaluation-round3-2026-03-08.md`: principal-scoped policy and vector BLOB compatibility hardening.
- `docs/roadmap.md`: phased expansion plan.

## Quick Start

```bash
# 1) build + test
cargo test --workspace

# 2) initialize database
cargo run -p loong-memory-cli -- init --db ./loong-memory.db

# 3) write memory
cargo run -p loong-memory-cli -- put \
  --db ./loong-memory.db \
  --namespace agent-demo \
  --external-id profile \
  --content "Alice likes rust and sqlite" \
  --metadata '{"source":"seed"}' \
  --principal operator

# 4) read memory
cargo run -p loong-memory-cli -- get \
  --db ./loong-memory.db \
  --namespace agent-demo \
  --external-id profile

# 5) recall
cargo run -p loong-memory-cli -- recall \
  --db ./loong-memory.db \
  --namespace agent-demo \
  --query "rust sqlite" \
  --limit 3

# 6) audit trail
cargo run -p loong-memory-cli -- audit --db ./loong-memory.db --limit 20
```

## Verification Gates

Local quality baseline:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Security and Reliability Defaults

- Namespace is required in every operation.
- Policy is checked before store access.
- All actions emit auditable events (allow/deny + operation detail).
- Store operations use transactions for consistency.
- SQLite `busy_timeout` and WAL are enabled for concurrency resilience.
- recall has an explicit upper bound (`max_recall_limit`) to prevent abusive scans.

## License

MIT OR Apache-2.0
