# OneContext Reverse Engineering Notes (Local Installation)

Date: 2026-03-08
Scope: local installed `onecontext` runtime under `/Users/chum/.local/share/uv/tools/aline-ai/`

## 1. Why This Research Matters

The target of `loong-memory` is not only storage. It must support a real agent workflow:

- context scoping
- incremental history ingestion
- robust search over conversation artifacts
- strict reproducibility under operations pressure

`onecontext` is a mature reference for this workflow shape.

## 2. Reverse-Engineered Entry Path

### CLI entry

`onecontext` is a Python entry script delegating to:

- `realign.cli:app`

### Relevant command implementation

- `realign/commands/context.py`
  - resolves current scope via `ALINE_AGENT_ID`
  - fallback to recent sessions when env is not set
- `realign/commands/search.py`
  - supports search types: `all`, `turn`, `session`, `content`
  - regex-first matching with pagination windows (`--from`, `--to`)
  - scope filters by agent->session mapping

### Data backend characteristics

- SQLite backend with explicit `REGEXP` function registration
- WAL mode, busy timeout, and query-only mode for read scenarios
- indexed tables for sessions/turns/content relation and agent context links

## 3. Core Implementation Logic (Observed)

## 3.1 Scope Resolution First, Search Second

The system resolves effective agent scope before querying. This reduces accidental cross-agent leakage and keeps search latency bounded.

Design lesson adopted in `loong-memory`:

- namespace is mandatory and embedded in every query path.
- selector ambiguity is rejected (`id` XOR `external_id`).

## 3.2 Multi-Granularity Search Modes

OneContext separates search units by semantic granularity:

- `session`: coarse narrative index
- `turn`: medium granularity summaries
- `content`: deep raw transcript search

Design lesson adopted in `loong-memory`:

- hybrid retrieval is built as candidate-stage composition (lexical + vector) then weighted merge.
- retrieval path stays deterministic with explicit tie-breakers.

## 3.3 Windowed Pagination and Count Awareness

Search output explicitly reports total matched count and current window, guiding iterative deep-dive workflows.

Design lesson adopted in `loong-memory`:

- recall candidate scans are bounded.
- result sorting is stable and reproducible.
- API surfaces explicit limit control.

## 3.4 Durable Event Pipeline Mindset

OneContext runtime includes hooks and worker-oriented ingestion signals. Even in CLI paths, the architecture anticipates asynchronous processing and eventual consistency.

Design lesson adopted in `loong-memory`:

- contracts are split (store/embedder/policy/audit) to allow async or distributed evolution without breaking call sites.
- durable audit is treated as a first-class stream, not debug text.

## 4. What We Did Not Copy (Intentionally)

- We did not replicate onecontext's full watcher/worker/TUI orchestration in Phase 1.
- We did not copy their exact schema or agent metadata model.

Reason:

- Phase 1 target is kernel-grade memory core + CLI.
- orchestration belongs to Phase 2 daemon and control-plane expansion.

## 5. Mapping to loong-memory Architecture

OneContext concept -> loong-memory implementation:

- Scope isolation -> required namespace + policy gate + query filter
- Search mode separation -> lexical/vector candidate separation + merge stage
- Operational durability -> SQLite WAL + transaction boundaries + busy timeout
- Observability -> persistent `memory_audit` and inspectable CLI audit command

## 6. Risks and Future Work

- Current vector persistence is JSON text for portability; Phase 2 may move to compact binary format.
- Current lexical query builder intentionally sanitizes tokens; advanced query syntax needs guarded expansion.
- Current policy is pluggable but default simple; richer multi-principal policy context is a next-phase requirement.

## 7. Conclusion

Reverse engineering confirmed that the most reusable onecontext kernel ideas are:

- scope-first isolation
- staged retrieval pipeline
- explicit pagination/control semantics
- durable operational events

These are now concretely embodied in `loong-memory` Phase 1 implementation.
