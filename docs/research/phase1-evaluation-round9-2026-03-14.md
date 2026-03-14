# Phase 1 Deep Evaluation Round 9

Date: 2026-03-14
Scope: CLI policy control plane and audit hardening

## 1. Objective

Round8 moved maintenance commands behind the engine policy boundary. A follow-up
review showed that the operator-facing CLI still had a material control-plane
gap:

- `audit` read directly through `SqliteAuditLog`, bypassing engine policy flow
- CLI always used `AllowAllPolicy`, so principal-scoped policy support was not
  operationally usable from the shipped binary
- audit persistence remained fail-open and SQLite audit storage was not
  append-only

This round closes those gaps and aligns the shipped CLI with the repository's
documented security model.

## 2. Findings

Pre-change risk:

- audit history could be read without engine-level authorization semantics
- audit event writes could fail silently
- duplicate audit event IDs could overwrite history
- CLI policy support existed in the core model but not in the operator surface

## 3. Implementation

## 3.1 Audit Contract Hardening

- `AuditSink::record` is now fallible and returns `Result<(), LoongMemoryError>`
- SQLite audit persistence now uses strict `INSERT`
- duplicate audit event IDs surface as storage errors instead of replacing rows

## 3.2 Engine-Level Audit Read Path

Added `MemoryEngine::audit_events(ctx, namespace, limit)`:

- validates namespace
- evaluates `Action::AuditRead`
- reads namespace-scoped audit history from the configured audit sink
- emits allow/read audit events after collecting the result

This preserves useful history output by excluding self-generated `audit` events
from the returned payload.

## 3.3 Static Policy Config Support

Added JSON-deserializable static policy config in `loong-memory-core`:

- `StaticPolicyConfig`
- `PrincipalNamespaceActionsConfig`
- `StaticPolicy::from_config(...)`

`Action` now deserializes using snake_case names such as `audit_read`.

## 3.4 CLI Policy Control Plane

`loong-memory-cli` now supports:

- global optional `--policy-file <path>`
- JSON static policy loading into `StaticPolicy`
- namespace-scoped, principal-required `audit`
- engine-routed audit reads instead of direct `SqliteAuditLog` bypass

CLI behavior is intentionally preserved for local development when no
`--policy-file` is supplied: policy defaults to `AllowAllPolicy`.

## 4. Verification

Focused tests added/updated:

- `audit_sqlite_integration::sqlite_audit_sink_rejects_duplicate_event_ids`
- `engine_store_integration::put_surfaces_post_write_audit_failure`
- `engine_store_integration::audit_read_without_reader_is_not_implemented`
- `engine_store_integration::audit_read_is_policy_gated_and_emits_denied_event`
- `engine_store_integration::audit_read_excludes_self_generated_audit_events_from_results`
- `policy_static_tests::static_policy_config_supports_snake_case_actions`
- `cli_policy_integration::audit_command_requires_principal`
- `cli_policy_integration::policy_file_allows_put_and_audit_for_operator`
- `cli_policy_integration::policy_file_denies_audit_without_audit_read_permission`

Planned full quality gates for final verification:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`

## 5. Impact

Security:

- closes the audit-read CLI bypass path
- makes principal-scoped static policy enforceable from the shipped binary

Auditability:

- audit writes are no longer silently best-effort
- SQLite audit history is append-only
- `audit` output remains useful because self-generated read events are excluded

Delivery quality:

- README, architecture, example policy, and CLI behavior now describe the same
  control-plane model

## 6. Residual Risk

- CLI still defaults to `AllowAllPolicy` when no `--policy-file` is supplied;
  this is intentional for local ergonomics but must be understood by operators.
- audit persistence failures can now surface after a store mutation has already
  committed; callers should inspect state before retrying after audit-related
  errors.
- static policy remains a simple allow-list model; richer policy engines remain
  future work.
