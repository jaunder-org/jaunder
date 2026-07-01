# ADR-0017: Error Handling — Typed Domain Errors, a Masked Public Boundary, and Typed Internal Sources

- Status: accepted
- Deciders: mdorman, Claude Opus
- Date: 2026-06-13

## Context and Problem Statement

Jaunder has two error "ladders":

- `WebError` (`web/src/error.rs`) — the **public**, serializable error returned
  to clients (web + API). It crosses the wire.
- `InternalError` (`web/src/error.rs`, `#[cfg(feature = "ssr")]`) — the
  **operator-side** error used inside `#[server]` functions; it never crosses
  the wire.

Beneath them, the storage and `common` crates define typed domain error enums
(`UserAuthError`, `UpdatePostError`, `PerformCreationError`, `MailError`,
`RegenerateError`, …) with `thiserror`.

Two recurring failure modes motivated recording a durable policy:

1. **Leakage.** Public constructors `WebError::storage(err)` /
   `WebError::server(err)` embedded the full `error_with_sources(err)` chain —
   raw DB/driver text — into a client-visible message. They were never used at a
   production boundary (boundaries already mask via `InternalError`), but they
   were reachable by accident. (analysis §2.4)
2. **Lossy stringification.** Several "catch-all" internal variants flattened a
   structured source into `Display` text at the point of failure
   (`Internal(String)`, `Send(String)`, `Storage(String)`, …), destroying the
   `sqlx::Error` kind / SQLSTATE / source chain needed to classify and triage
   failures. (analysis §3.1-A)

This ADR records the error-handling conventions that are now **decided and
implemented**. The further structural reshape of the internal carrier — a
`kind`/`ErrorClass`/`context` structure with field emission at the boundary
(analysis §3.1-B/C, tracked as `jaunder-kq8w.16`) — is **forthcoming** and will
build on, not replace, this ADR.

## Decision Drivers

- Make invalid states unrepresentable; parse and reject at boundaries (project
  conventions).
- **Never leak internal error detail to a client.**
- **Preserve the typed cause to the boundary** so failures can be classified
  (infra vs. bug vs. client vs. external) — see ADR-0011 (observability).
- Prefer widely-understood tooling (`thiserror`) over bespoke machinery.
- Make the safe path the easy path: remove footguns rather than document around
  them.

## Decision Outcome

### 1. Domain errors stay typed (`thiserror`)

Expected failures are modeled as discrete variants in `storage`/`common` enums
and mapped outward to specific `WebError`/HTTP responses. We do **not** collapse
domain errors into an opaque carrier; matching on `NotFound` vs. `Unauthorized`
vs. `SlugConflict` must remain possible.

### 2. The public boundary masks; nothing else may leak

- `WebError` carries no raw internal text. Storage/server failures reach a
  client **only** through `InternalError`, which masks the public side
  (`"storage operation failed"` / `"server operation failed"`) while retaining
  operator detail for logs.
- `server_boundary` logs the operator detail and returns the masked public
  error.
- The leaky public constructors `WebError::storage`/`WebError::server` are
  **removed**; `error_with_sources` is `pub(crate)` so external code cannot
  build a leaky message with it. (§2.4)
- **Invariant:** internal error detail reaches a client only via the
  `InternalError` masking boundary. There is no public API surface that
  serializes a raw source chain into `WebError`.

### 3. Internal error variants preserve a _typed_ source, never a stringified one

At the point of failure, do not flatten the cause to a `String`. Instead:

- **Single concrete source** → carry it via `#[from]` / `#[source]` (e.g.
  `PerformCreationError::InvalidSlug(#[from] InvalidSlug)`,
  `…::Storage(#[source] sqlx::Error)`).
- **Heterogeneous sources** (a variant that legitimately wraps several unrelated
  error types) → carry `#[source] Box<dyn std::error::Error + Send + Sync>`. The
  boundary can still `downcast_ref::<sqlx::Error>()` to classify (SQLSTATE /
  pool timeout / I/O). Examples shipped in §3.1a: `UserAuthError::Internal`
  (sqlx + `io` + record-conversion) and `MailError::Send` (lettre address/SMTP +
  `serde_json` + file-capture `io`).
- **Context, not a cause** → when there is genuinely no underlying error object
  (e.g. a value that failed an `Option`-returning parse), carry the offending
  _value_ as context rather than inventing a source. Example:
  `RegenerateError::BadUrl(String)` holds the unparseable URL.

The `Box<dyn Error>` choice for heterogeneous variants is deliberate: it
preserves `source()` for every site and stays `downcast_ref`-able, without
forcing an artificial sub-enum or pulling the domain error into `anyhow` (which
would erase the typed outward mapping in #1).

## Forthcoming (not decided here)

The structured internal carrier — replacing `InternalError`'s flat
`operator_message: String` with `kind` / `ErrorClass` / `context` and emitting
them as discrete fields at the boundary, with `anyhow` as the operator-context
carrier — is analysis §3.1-B/C and is tracked as `jaunder-kq8w.16`. It extends
this ADR; the typed sources preserved per #3 are exactly what it will classify.

## Consequences

- Good: no client-facing leakage of DB/driver internals; operator logs retain
  full cause chains; sources remain typed and downcastable for classification;
  tooling stays familiar (`thiserror`).
- Cost: `Box<dyn Error>` erases the static type for heterogeneous variants —
  mitigated by `downcast_ref` at the boundary. The masking boundary must be used
  at every server function — enforced structurally by removing the leaky
  constructors.
- The typed-source convention (#3) must be **preserved through the
  SQLite/Postgres storage dedup** (analysis §1.1, `jaunder-kq8w.3`); the
  `authenticate` source-preservation in particular is noted there so the merge
  does not silently re-stringify.
