# loong-memoryd Auth Envelope Design

Date: 2026-03-15
Owner: chumyin
Issue: #8

## 1. Context

`loong-memoryd` now exposes a meaningful HTTP surface and already delegates
authorization to the core `MemoryEngine`. Protected routes currently require:

- `x-loong-principal`

That gives the engine a principal string for policy evaluation, but it does not
authenticate that principal. Any caller who can reach the daemon can claim any
principal name.

## 2. Problem Statement

The daemon currently has authorization without real transport authentication.

That is an important gap because the project now advertises an HTTP daemon as
the Phase 2 runtime surface. Once a network boundary exists, "the client told us
who they are" is no longer a sufficient identity model.

The next high-value step is not OAuth or external identity integration. It is a
minimal but real authentication envelope that proves which principal is being
used for policy decisions.

## 3. Goals

1. Add an optional transport authentication mode for `loong-memoryd`.
2. Authenticate callers with `Authorization: Bearer <token>` when auth mode is
   enabled.
3. Derive the effective principal from the configured token mapping rather than
   trusting `x-loong-principal`.
4. Preserve today's trusted-header mode when no auth config is supplied so
   local operator ergonomics remain intact.
5. Add end-to-end daemon tests for success, missing token, invalid token, and
   spoofed principal-header rejection.
6. Update docs so the daemon's authn/authz story matches shipped behavior.

## 4. Non-Goals

- OAuth/OIDC
- external identity providers
- token minting, rotation, or revocation APIs
- dynamic auth reload
- replacing or duplicating core policy logic
- per-token authorization rules in the transport layer

## 5. Approaches Considered

### Approach A: Keep Trusted Header Mode Only

Continue requiring `x-loong-principal` and document that the daemon is intended
only for trusted local callers.

Pros:

- zero new config format
- preserves the smallest possible daemon

Cons:

- does not actually improve transport authentication
- weakens the credibility of the roadmap's authn/authz envelope claim
- leaves principal spoofing trivial

### Approach B: Optional Static Bearer Token Auth

Add an optional auth file mapping static bearer tokens to principals. When the
file is configured, protected routes require `Authorization: Bearer <token>` and
the daemon derives principal from that mapping.

Pros:

- smallest real authentication step
- simple to document and test
- reuses the existing policy engine cleanly
- preserves local trusted-header ergonomics when auth is not configured

Cons:

- static secret management only
- no rotation or external identity integration

### Approach C: Jump Straight to OAuth/OIDC or mTLS

Integrate a fully externalized identity boundary immediately.

Pros:

- stronger long-term security model

Cons:

- far too much scope for the current repository phase
- large dependency and operator-complexity jump
- delays useful incremental hardening

## 6. Chosen Approach

Approach B.

The repository needs a real authentication envelope now, not a complete
enterprise identity system. Optional static bearer auth is the right
middle-ground for this phase.

## 7. Proposed Design

## 7.1 Configuration Model

Add an optional daemon CLI flag:

- `--auth-file <path>`

The auth file contains static token-to-principal mappings:

```json
{
  "tokens": [
    {
      "token": "operator-secret",
      "principal": "operator"
    },
    {
      "token": "viewer-secret",
      "principal": "viewer"
    }
  ]
}
```

Validation requirements:

- token must be non-empty
- principal must be non-empty
- duplicate tokens are rejected

## 7.2 Runtime Modes

The daemon should support two explicit auth modes:

### Trusted Header Mode

Used when `--auth-file` is absent.

Behavior:

- preserve today's `x-loong-principal` requirement
- intended for local or otherwise trusted operator environments

### Static Token Mode

Used when `--auth-file` is present.

Behavior:

- protected routes require `Authorization: Bearer <token>`
- bearer token maps to the effective principal
- `x-loong-principal` is ignored for identity derivation

This prevents callers from spoofing a more privileged principal in the header
while using a token that belongs to a less privileged identity.

## 7.3 Auth State

Extend `ServiceState` with transport-auth configuration:

- `auth_mode: AuthMode`

Proposed enum:

- `TrustedHeader`
- `StaticToken { token_to_principal }`

The token map should be immutable in runtime state.

## 7.4 Health Response

Extend `/healthz` with:

- `auth_mode`

Proposed values:

- `trusted_header`
- `static_token`

This makes operational state visible without exposing secrets.

## 7.5 Error Semantics

When auth mode is `StaticToken`:

- missing `Authorization` header -> `401`, `missing_authentication`
- malformed `Authorization` value -> `401`, `invalid_authentication`
- unknown token -> `401`, `invalid_authentication`

When auth mode is `TrustedHeader`:

- missing `x-loong-principal` -> existing `401`, `missing_principal`

Health remains unauthenticated.

## 7.6 Policy Interaction

Transport authentication should only establish principal identity.
Authorization remains unchanged:

1. daemon authenticates request and derives principal
2. daemon constructs `OperationContext`
3. `MemoryEngine` enforces policy for that principal

This keeps transport auth and core authorization cleanly separated.

## 7.7 Test Strategy

Add end-to-end tests for:

- trusted-header mode still works when `--auth-file` is absent
- static-token mode allows requests with valid bearer token
- static-token mode rejects missing bearer token
- static-token mode rejects invalid bearer token
- static-token mode ignores spoofed `x-loong-principal`
- `/healthz` reports `auth_mode`

At least one spoofing test should prove that:

- token maps to `viewer`
- request sends `x-loong-principal: operator`
- policy allows `operator` but not `viewer`
- request is denied

That is the most important regression test in the whole slice.

## 7.8 Documentation Updates

Update:

- `README.md`
- `docs/architecture.md`
- `docs/research/phase2-service-evaluation-2026-03-15.md`

The docs should describe:

- trusted-header fallback mode
- static bearer-token auth mode
- principal derivation from bearer token
- auth_mode visibility in health output

## 8. Risks and Mitigations

Risk: token auth creates a second policy system by accident.

Mitigation:

- auth config only maps token -> principal
- all authorization stays inside the existing policy engine

Risk: callers keep assuming `x-loong-principal` matters in token mode.

Mitigation:

- document that static-token mode ignores caller-supplied principal headers
- add a regression test that proves spoofing does not work

Risk: the daemon becomes harder to use locally.

Mitigation:

- preserve current trusted-header mode when `--auth-file` is absent
- keep the new auth mode opt-in
