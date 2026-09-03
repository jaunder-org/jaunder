# Issue #859 — return 400 for malformed server-function arguments

## Outcome

A request whose typed `#[server]` arguments cannot be decoded receives HTTP 400
Bad Request instead of HTTP 500 Internal Server Error. The public
`WebError::ServerFunction` payload and message remain compatible, while genuine
server-function and response failures continue to return 500.

## Load-bearing decisions

- Jaunder owns this correction locally. The change does not fork, patch, or wait
  on `server_fn`, because upstream acceptance and release timing are uncertain.
- Classification is structural, never inferred from human-readable messages.
  Framework `Args`, `MissingArg`, and input-side `Deserialization` errors are
  malformed client input. Function-body errors, output-side `Serialization`, and
  every other internal failure remain server errors.
- `WebError` preserves that typed classification only across the internal
  framework response hop. One framework-wide `/api` response normalizer consumes
  it, selects HTTP 400, and removes it before the response crosses the public
  boundary.
- The normalizer is the sole status-policy seam. Individual server functions,
  domain newtypes, fields, and codecs do not acquire response-status logic.
- The client-facing error representation remains
  `WebError::ServerFunction { message }`; the internal classification must not
  alter or leak into its serialized body.
- `leptos_axum` may turn a progressive-enhancement form error into a temporary
  redirect before the normalizer runs. For a structurally classified malformed
  request, the normalizer restores 400 and removes that `Location`; valid form
  redirects remain unchanged.
- Client-side validation remains the primary browser UX and server-side typed
  decoding remains defense in depth. Client validation alone is not accepted as
  the fix because direct HTTP clients must receive the correct status.
- ADR-0065 and its architecture projection record the corrected decode-stage
  consequence. This does not introduce new ubiquitous language, so `CONTEXT.md`
  remains unchanged.

## Acceptance

- Malformed URL/form-encoded typed arguments return HTTP 400 through the real
  `/api/{fn}` routing stack on both supported storage backends.
- Malformed JSON typed arguments return HTTP 400 through that same routing
  stack.
- Missing typed arguments return HTTP 400 wherever `server_fn` classifies them
  as `MissingArg` or `Args`.
- Existing representative failures retain the same public
  `WebError::ServerFunction` variant and message they expose today; no internal
  classification field or variant appears on the wire.
- A malformed progressive-enhancement form request returns 400 without a
  redirect `Location`, while an existing valid form redirect still returns its
  established status and destination.
- A representative error raised after successful argument decoding remains HTTP
  500 with its established public body.
- Direct structural-classification proof shows output-side `Serialization` is
  not tagged as malformed input and remains HTTP 500. If production behavior can
  trigger it naturally, the real routing stack also exercises that response;
  production code gains no test-only failure path.
- Existing malformed `PostBody`, `PostFormat`, and inbound-password request
  regressions assert 400 rather than 500, demonstrating that the policy is not
  tied to one domain type or vertical.
- ADR-0065 and `docs/ARCHITECTURE.md` describe the 400/500 classification and
  the internal-tag removal invariant.

## Boundaries

- No `server_fn` or `leptos_axum` fork, Cargo patch, vendored framework code, or
  upstream contribution is required.
- No copied replacement for the `leptos_axum` server-function handler.
- No per-field, per-newtype, per-codec, or per-server-function response mapping.
- No parsing of error display strings or domain validation messages.
- No redesign of the public error body, client-side validation UX, or telemetry
  classification.
- No dependency upgrade undertaken solely for this issue.
- No new endpoint or browser interaction flow; existing browser flows and valid
  progressive-enhancement redirects remain behaviorally unchanged.
