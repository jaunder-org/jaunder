# ADR-0072: Timestamps cross the web boundary as a `UtcInstant` newtype (chrono is already in the wasm bundle)

- Status: proposed
- Date: 2026-07-19
- Issue: [#91](https://github.com/jaunder-org/jaunder/issues/91)

## Context

ADR-0063 established the domain-value newtype convention with two backing
flavors: **string-backed** (the `StrNewtype` trailer + validating serde bridge)
and **numeric-ID** (the `IdNewtype` transparent-i64 bridge). It explicitly names
#91 — "typed timestamps … out through web DTOs and the `#[server]` boundary" —
as a target, but leaves the mechanism open, because a timestamp fits neither
existing flavor.

Timestamps crossed the web `#[server]` boundary as bare RFC3339 `String` /
`Option<String>` at ~25 sites (8 params incl. `publish_at` and six pagination
`cursor_created_at`; 17 return-DTO fields such as `DraftSummary.created_at` /
`scheduled_at`). Parsing/validation lived at the edge
(`web/src/posts/mod.rs::parse_publish_at`).

The stated reason (recorded for `publish_at` in #70) was that **`chrono` is an
`ssr`-only dependency in `web`**, so a `DateTime<Utc>` in a `#[server]`
signature — a type named by the generated client stub — would not compile for
the wasm client.

**That reason is obsolete, and this ADR records why.** `chrono` is already
compiled into the CSR/wasm bundle today, via the unconditional
`web → common → chrono` chain: `common` depends on `chrono` un-gated
(`chrono.workspace = true`) and uses `DateTime<Utc>` in public API
(`common::feed`), and `web` depends on `common` un-gated in every build
including `csr`. chrono 0.4's default `wasmbind` feature makes it wasm-clean
with no extra feature work. The `web`-level `chrono = { optional, server-only }`
gate only keeps `web`'s _own_ `use chrono` sites out of the CSR feature set — it
never kept the crate out of the bundle. (The client is CSR / `mount_to_body`,
not hydrate — ADR-0040.) ADR-0056 already anticipated running
chrono-with-`wasmbind` in the browser.

## Decision

**Timestamps that cross the web boundary are a domain newtype,
`common::time::UtcInstant`, wrapping `chrono::DateTime<Utc>`** — a third,
**instant-backed** flavor of the ADR-0063 convention, alongside string-backed
and numeric-ID.

Because neither the `StrNewtype` nor `IdNewtype` derive fits a `DateTime`
backing, `UtcInstant`'s trailer is **hand-written**, and defined by these rules:

- **Wire form is RFC 3339, via chrono's own `serde`.** `common` enables chrono's
  `serde` feature (a one-line addition — chrono is already a `common`
  dependency), so `DateTime<Utc>: Serialize/Deserialize` is available in every
  build including CSR. `UtcInstant` derives `Serialize`/`Deserialize`; as a
  serde-transparent newtype it (de)serializes exactly as `DateTime<Utc>` — an
  RFC 3339 string, wire-compatible with the prior String fields — and chrono's
  `Deserialize` normalizes any offset to UTC and rejects malformed input,
  preserving decode-time wire validation with no hand-written bridge. Reusing
  the already-available impl is preferred over re-implementing it; the
  dependency on the feature is not a silent-failure risk, because the pre-push
  e2e build compiles the CSR/wasm target and would fail loudly were the feature
  ever disabled. `FromStr` stays hand-written — it is the single
  validation/normalization chokepoint and is required for the client `Field`
  path (below).
- **A newtype, not a raw `DateTime<Utc>`**, because a timestamp is a domain
  value (this milestone's thesis) and the newtype is the one home for its
  `FromStr` validation, `Display`/formatting, and the ADR-0065 client `Field`
  hook. That chrono is already in the wasm bundle is what makes the typed
  instant expressible at the boundary at all.
- **`FromStr` canonicalizes to UTC** (`.with_timezone(&Utc)`), so an
  offset-bearing input is stored as the equivalent UTC instant and re-serializes
  in canonical UTC RFC 3339 form. Its `Err` implements `Display`, so
  `UtcInstant` drops directly into the ADR-0065 `Field<T>` / `ValidatedInput<T>`
  client-validation machinery for the one _input_ timestamp (`publish_at`).
- It is **not a secret and not a bearer token** — it takes no `Proffered*` twin
  and needs **no new xtask gate**; it is usable directly as both `#[server]` arg
  and return, like `Slug`/`Username`.

The browser's local→UTC wall-clock conversion for the datetime-local control
stays a **browser** concern (`js_sys::Date`), orthogonal to chrono; its RFC 3339
output parses into `UtcInstant`. Migrating that helper to chrono is ADR-0056's
separate scope.

## Consequences

- Every timestamp on the web boundary is validated in one place (`UtcInstant`),
  not at the edge; `parse_publish_at` and per-field `.to_rfc3339()` marshalling
  are deleted. A malformed `publish_at` now fails in arg-decode (before the fn
  body), so legitimate clients must pre-validate — which they do, via
  `Field<UtcInstant>` (ADR-0065).
- Client code may now format and compute on timestamps with chrono directly (it
  is already in the bundle), retiring ad-hoc string handling in
  `format_post_time` and friends.
- Commits us to `chrono` remaining a wasm-built dependency of `common` **with
  its `serde` feature enabled**. The wasm-bundle cost is already paid for chrono
  itself (measured via `cargo xtask audit-wasm`; expected ≈ 0 delta); the added
  serde impls are negligible. The feature-enablement is guarded structurally:
  the CSR/wasm target won't compile without it, and the pre-push e2e build
  compiles that target — so a regression is a loud build failure, never a silent
  one.
- Establishes the instant-backed newtype as the pattern for any future
  wall-clock value crossing a boundary. Does **not** change how storage or
  `common`/`host` internals carry time (still raw `DateTime<Utc>`); the newtype
  is a boundary/DTO type.
- Does not address the other weak boundary primitives (post `format`, `summary`,
  `bio`, tokens, URLs) — those have their own milestone-13 issues.
