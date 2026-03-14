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
  - JSON-loadable static policy config support
  - vector persistence as compact BLOB with legacy JSON read compatibility
  - fallible, append-only audit sink contracts
- `loong-memory-cli`
  - `init`, `put`, `get`, `recall`, `delete`, `audit`, `vector-health`, `vector-repair`
  - global optional `--policy-file <path>` for CLI policy enforcement
  - namespace-scoped, policy-gated `audit` reads with explicit principal

## Repository Layout

- `crates/loong-memory-core`: core contracts and engine implementation.
- `crates/loong-memory-cli`: command-line operational entrypoint.
- `docs/architecture.md`: architecture and data model details.
- `docs/examples/static-policy.example.json`: example JSON policy file for CLI enforcement.
- `docs/research/onecontext-reverse-engineering.md`: onecontext implementation analysis and extracted design lessons.
- `docs/research/phase1-evaluation-2026-03-08.md`: deep evaluation, optimization decisions, and verification results.
- `docs/research/phase1-evaluation-round2-2026-03-08.md`: multilingual retrieval hardening and recall-bound enforcement.
- `docs/research/phase1-evaluation-round3-2026-03-08.md`: principal-scoped policy and vector BLOB compatibility hardening.
- `docs/research/phase1-evaluation-round4-2026-03-08.md`: vector integrity validation and corruption-resilient recall hardening.
- `docs/research/phase1-evaluation-round5-2026-03-08.md`: startup migration v2 for legacy text vectors and migration resilience tests.
- `docs/research/phase1-evaluation-round6-2026-03-08.md`: vector health diagnostics API/CLI and integrity observability tests.
- `docs/research/phase1-evaluation-round7-2026-03-08.md`: vector repair API/CLI (`dry-run` + `apply`) and repair integrity tests.
- `docs/research/phase1-evaluation-round8-2026-03-09.md`: maintenance command security hardening (policy/audit gated vector health/repair).
- `docs/research/phase1-evaluation-round9-2026-03-14.md`: CLI policy control-plane and audit hardening evaluation.
- `docs/plans/2026-03-14-cli-policy-control-plane-design.md`: design for CLI policy/audit control-plane hardening.
- `docs/plans/2026-03-14-cli-policy-control-plane.md`: implementation plan for CLI policy/audit control-plane hardening.
- `docs/roadmap.md`: phased expansion plan.

## Quick Start

```bash
# 1) build + test
cargo test --workspace

# 2) initialize database
cargo run -p loong-memory-cli -- init --db ./loong-memory.db

# 3) optionally enforce policy with a JSON config
cp docs/examples/static-policy.example.json ./policy.json

# 4) write memory
cargo run -p loong-memory-cli -- --policy-file ./policy.json put \
  --db ./loong-memory.db \
  --namespace agent-demo \
  --external-id profile \
  --content "Alice likes rust and sqlite" \
  --metadata '{"source":"seed"}' \
  --principal operator

# 5) read memory
cargo run -p loong-memory-cli -- --policy-file ./policy.json get \
  --db ./loong-memory.db \
  --namespace agent-demo \
  --external-id profile \
  --principal operator

# 6) recall
cargo run -p loong-memory-cli -- --policy-file ./policy.json recall \
  --db ./loong-memory.db \
  --namespace agent-demo \
  --query "rust sqlite" \
  --limit 3 \
  --principal operator

# 7) audit trail (namespace + principal are required)
cargo run -p loong-memory-cli -- --policy-file ./policy.json audit \
  --db ./loong-memory.db \
  --namespace agent-demo \
  --limit 20 \
  --principal operator

# 8) vector health diagnostics
cargo run -p loong-memory-cli -- --policy-file ./policy.json vector-health \
  --db ./loong-memory.db \
  --namespace agent-demo \
  --invalid-sample-limit 20 \
  --principal operator

# 9) vector repair (dry-run by default, add --apply to write changes)
cargo run -p loong-memory-cli -- --policy-file ./policy.json vector-repair \
  --db ./loong-memory.db \
  --namespace agent-demo \
  --issue-sample-limit 20 \
  --principal operator
```

## Verification Gates

Local quality baseline:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Security and Reliability Defaults

- Namespace is required in every operation, including `audit`.
- CLI defaults to `AllowAllPolicy` when `--policy-file` is omitted.
- Policy is checked before store access.
- Static JSON policy files can grant namespace-level and principal+namespace actions.
- All actions emit auditable events (allow/deny + operation detail).
- Audit reads are policy-gated and returned history excludes self-generated audit-read events.
- Audit persistence surfaces write failures instead of silently dropping events.
- SQLite audit persistence is append-only; duplicate audit event IDs fail instead of replacing history.
- Store operations use transactions for consistency.
- SQLite `busy_timeout` and WAL are enabled for concurrency resilience.
- recall has an explicit upper bound (`max_recall_limit`) to prevent abusive scans.

## Phase 1 Audit Semantics

- Store writes and audit persistence are still decoupled in Phase 1.
- A post-operation audit failure can therefore be returned after the store
  mutation has already committed.
- This release intentionally surfaces that failure instead of silently losing the
  audit event; callers should inspect state before retrying after audit-related
  errors.

## License

MIT
