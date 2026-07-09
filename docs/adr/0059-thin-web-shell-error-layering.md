# ADR-0059: The thin web shell and the T1→T2→T3 error pipeline

- Status: proposed
- Date: 2026-07-09
- Issue: [#334](https://github.com/jaunder-org/jaunder/issues/334)

## Context

`web/` had accreted `#[cfg(feature = "server")]` code with no intrinsic tie to
leptos, wasm, or the server-fn boundary — a discipline leak. The clearest
instance is the error layer. Three error tiers exist, but two of the three lived
in `web/`, the least general place:

- **T1 — typed domain errors** (`AudienceError`, `UserAuthError`, …): the
  discrete, expected failures of one storage/`common` operation, matched on by
  callers (ADR-0017 §1). Correctly placed in `storage`/`common`.
- **T2 — the operator carrier** (`InternalError` + `ErrorKind`/`ErrorClass` +
  boundary logging/metrics): general server-error machinery that never crosses
  the wire. Lived in `web`, tied there by a single field: `public: WebError`.
- **T3 — the wire type** (`WebError`): serialized, crosses the network,
  deserialized by the wasm client; impls leptos's `FromServerFnError`. Genuinely
  `web`.

ADR-0017 recorded the masked-boundary and typed-source conventions and
explicitly left the "structured internal carrier"
(`kind`/`ErrorClass`/`context` + `anyhow` source) as _forthcoming_. That carrier
now exists as T2 — and this ADR moves it to its correct home and records the
principle behind the whole cleanup.

## Decision

### The invariant: `web/` is a thin shell

> For any `#[cfg(feature = "server")]` item in `web/`, ask: **can it literally
> not live anywhere else?** If it can — because it is plain effectful/host logic
> with no leptos, wasm, or wire-type tie — it is in `web/` by accretion, not
> necessity, and it moves down to where it is host-testable
> (`host`/`storage`/`common`).

Only three kinds of thing survive that test as genuinely `web`: the leptos UI
(`#[component]`), the `#[server]` surface, and the server-fn **wire types**
(`WebError` and the `Ok`-side response DTOs). Everything else — the error
carrier, per-vertical error mapping, guards, effectful helpers — is pushed down
as its vertical is touched (the same as-we-go model as #303).

### Why the three tiers exist and stay separate

The tiers are a one-way **conversion** pipeline `T1 → T2 → T3` — no tier
_contains_ another; each is _converted into_ the next. Two boundaries, each
justified by a guarantee that collapsing it would forfeit. This is the
load-bearing rationale; do not re-derive it away.

- **T2 ↔ T3 is a security boundary made structural.** T3 crosses the network to
  an _untrusted client_, so it must be serializable/wasm-compatible **and**
  carry only client-safe data. T2 must retain the full operator payload — the
  typed source chain (DB/driver text, SQLSTATE), operator context,
  classification — none of which may reach a client (ADR-0017's leak failure
  mode). One type cannot satisfy both: merged, it either leaks operator detail
  over the wire, or drops the cause before it can be logged. Splitting them
  makes the masking boundary a **type** boundary — T2→T3 is a lossy one-way
  projection, so the operator payload is _structurally absent_ from the type
  that crosses the wire. Leakage is impossible **by construction, not by
  discipline**. Homes follow from the same fact: T3 lives with leptos/wasm in
  `web`; T2 is target-agnostic host machinery in `host`.

- **T1 ↔ T2 is typed discrete cause vs uniform carrier.** T1 keeps each
  operation's discrete _expected_ failures typed, so callers map each
  exhaustively (`NotFound→404`, `Conflict→409`; invalid states unrepresentable,
  ADR-0017 §1). T2 is the single uniform thing the boundary classifies, logs,
  and projects. Collapsing T1→T3 directly loses the typed cause + classification
  before logging (T3 cannot carry them), reintroducing ADR-0017's
  lossy-stringification failure. Collapsing T1 into T2 from the start loses
  compiler-checked exhaustive mapping of discrete failures.

### The carrier moves to `host`, decoupled from the wire type

T2 (`InternalError`, `ErrorKind`, `ErrorClass`, boundary logging + metric
emission) moves from `web` into `host::error`. It **drops** the
`public: WebError` field — its only web tie — carrying instead a masked
`public_message: String` alongside
`kind`/`class`/`context`/`source: Option<anyhow::Error>`. Construction is via
the typed constructors or `From<DomainError>`; **no constructor takes a
`WebError`**, so the T2→T3 projection
(`fn project(kind, &public_message) -> WebError`, which stays in `web`) is total
_by construction_.

The masked message lives in a **private** `public_message: String`, readable
only through a read-only accessor — there is no `pub` field — so the
client-facing string can only be _set_ by the vetted constructors (which always
mask), never injected from outside. This is the encapsulation half of "by
construction": the type-level split keeps the source chain out of the wire
_type_, and the private-field-plus-accessor keeps an un-masked string out of the
carrier's _public slot_.

Domain errors lift into the carrier via `From` impls **co-located with the
domain error**: `impl From<AudienceError> for host::error::InternalError` in
`storage` (orphan rule: `AudienceError` local there),
`From<MailError>`/`From<sqlx::Error>` in `host`. Every named per-vertical error
mapper (`map_audience_error`, …) collapses to `err.into()`.

**Recipe — lifting a typed error, by two cases (never a lossy `to_string()`):**

- **The error type has one canonical lift** (the public message is just its
  `Display`, no site context needed) → a `From` impl, co-located per the orphan
  rule; the `validation_from!` macro generates the family of value-object
  `Validation` lifts. This is _item_ generation — a macro's proper job — so the
  call site is a bare `?`.
- **The lift must be supplemented with site context** the error type doesn't
  carry (which field failed: `"invalid publish_at: {e}"`, an email parse) → the
  constructor `InternalError::validation_source(public_message, source)`. It is
  a call-site _expression_, so a constructor — not a macro — is the right tool:
  it fixes `kind=Validation`/`class=Client`, folds the
  `anyhow::Error::new(source)`, and lets the site keep the typed source without
  importing `anyhow`/`ErrorKind`/`ErrorClass`.

Both preserve the typed cause on the operator side; the difference is only
whether the public message comes from the error alone or needs supplementing.
`validation_source` is also the shared body the `validation_from!` macro and the
storage `From` impls delegate to, so the "masked-validation-with-source" shape
exists in exactly one place.

The `#[server]` boundary (`boundary!`) splits: the leptos owner-pinning half
(`ScopedFuture`, ancestor-owner hold — ADR-0016 #89/#124/#138) stays in `web`;
the log + metric half becomes a `host` carrier method. `web` keeps only
`WebError`, the projection, and the owner-pinning.

### `host` floor invariant (extends ADR-0058)

This cycle promotes `host` from a near-leaf utility crate to the host **floor**:
it gains `storage` and `web` as dependents.

> **`host` depends on no _workspace_ crate except `common`** — never
> `storage`/`web`/`server` (only workspace-crate deps can cycle or recreate the
> `AppState` omnibus). It **may** depend on external _infrastructure_ crates the
> carrier needs (`anyhow`, `tracing`, `sqlx`, `http`). The dividing line: `host`
> knows _raw infrastructure types_ (a `sqlx::Error` it classifies, `chrono`,
> HTTP headers) but **not** our domain/storage abstractions (`PostStorage`,
> `AudienceError`), which stay above it. That line is why `fetch_post_record`
> (uses `&dyn PostStorage`) homes in `storage` while `From<sqlx::Error>` (a raw
> driver type) homes in `host`.

`server` remains the host _ceiling_ (composition root, may know everything);
`host` is the _floor_.

### Naming

The carrier keeps the name `InternalError`, now `host::error::InternalError`. It
is still "internal" in the sense that matters — it never crosses the wire.

## Consequences

- Client-facing leakage of operator detail is now prevented by the type system,
  not by convention: the wire type structurally cannot hold the source chain.
- The carrier and its error-lift machinery are host-testable without any
  leptos/wasm entanglement; the per-vertical effectful helpers pushed down
  alongside it (auth credential/cookie logic, viewer identity, posts cursor/DTO
  orchestration) gain host/storage tests.
- Error handling is uniform: every _named_ domain/validation source lifts via a
  typed `From`; only the external `sqlx::Error` keeps a canonical `From` and
  `chrono` parse routes through the existing `masked(...)` (site-specific public
  message + `anyhow` source), so no lossy `.to_string()` of a typed source
  remains.
- `host` takes on external infra deps (`sqlx`/`http`) and two workspace
  dependents; ADR-0058's dependency rule is clarified to the
  _no-workspace-crate-above-it_ form.
- Out of scope, tracked separately: the UI co-location (#303/#314) and
  client-first form validation (#336). The thin-web-shell push-down of the
  remaining verticals is as-we-go, not a big-bang.
- Supersedes nothing; **extends** ADR-0017 (picks up its forthcoming-carrier
  thread) and ADR-0058 (the floor invariant); consistent with ADR-0016 (DI /
  anti-omnibus).
