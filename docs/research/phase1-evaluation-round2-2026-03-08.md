# Phase 1 Deep Evaluation Round 2

Date: 2026-03-08
Scope: deeper hardening after initial Phase 1 completion

## 1. Research Findings

This round focused on two higher-impact risks discovered during deep review:

1. language coverage risk:
- whitespace-only tokenization is weak for CJK and mixed-language text
- lexical path may return no candidates for non-space queries

2. resource-abuse risk:
- recall accepted arbitrarily large `limit`
- large limits can inflate merge/sort costs under load

## 2. Implemented Improvements

## 2.1 Multilingual tokenization kernel

Added shared tokenizer module:

- file: `crates/loong-memory-core/src/tokenize.rs`
- capabilities:
  - ASCII word token extraction
  - CJK unigram extraction
  - CJK bigram generation
  - dedupe + bounded token output

Applied to:

- embedding provider token stream
- FTS query term builder

## 2.2 Lexical fallback for CJK/non-space queries

When FTS lexical retrieval returns empty, store now runs a bounded fallback:

- scans recent namespace rows (bounded)
- computes token overlap score against query terms
- feeds fallback lexical score into hybrid merge

This prevents lexical channel from collapsing to zero in common CJK cases.

## 2.3 Recall upper-bound protection

Added engine-level control:

- `EngineConfig::max_recall_limit` (default `128`)
- request rejected when `limit` exceeds bound

## 2.4 Efficiency refinement

Recall record fetch path continues to use a single prepared statement per call.

## 3. Test-First Evidence

New/updated tests proved failures before fix and pass after fix:

- `recall_rejects_excessive_limit`
- `multilingual_cjk_recall_returns_relevant_record`
- tokenizer unit tests for ASCII and CJK bigrams

## 4. End-to-End Verification

Executed and passed:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`

Additional CLI smoke (Chinese query recall):

- namespace `zh-demo`
- query `内存检索`
- relevant Chinese memory ranked top in recall results

## 5. Residual Risks

- vector channel still uses deterministic hash embedding (not semantic model-grade).
- fallback lexical scoring is bounded and heuristic; future daemon phase can move to
  richer tokenizer/reranker pipeline.

## 6. Recommended Next Deep Step

- Introduce benchmark harness (`criterion`) for:
  - write throughput
  - recall latency vs `limit` and corpus size
  - multilingual query distributions
