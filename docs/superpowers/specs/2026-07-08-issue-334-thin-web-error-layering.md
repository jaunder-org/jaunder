# Spec — Issue #334: Thin web shell, first application: the error carrier / wire split

- Status: **draft** (pre-approval)
- Date: 2026-07-08
- Issue: [#334](https://github.com/jaunder-org/jaunder/issues/334)
- Blocked by: [#227](https://github.com/jaunder-org/jaunder/issues/227) (PR #335
  — introduces the `host` crate)
- Relates: #303 (web arc), #314 (audiences convergence — rebases on top of
  this), ADR-0016 (DI), ADR-0017 (error handling & the public boundary),
  ADR-0058 (host-crate layering)

## Principle (the corrective this issue records)

`web/` accreted `#[cfg(feature = "server")]` code that has no intrinsic tie to
leptos/wasm/the server-fn boundary — a discipline leak. The invariant, stated as
a test rather than a vibe:

> For any `#[cfg(feature = "server")]` item in `web/`, ask: **can it literally
> not live anywhere else?** If it _can_ — because it's plain effectful/host
> logic with no leptos, wasm, or wire-type tie — then it is in `web/` by
> accretion, not necessity, and it moves down to where it is host-testable
> (`host`/`storage`/`common`).

Only three kinds of thing survive that test and are _genuinely_ `web`: the
leptos UI (`#[component]`), the `#[server]` surface, and the server-fn **wire**
types. This spec records the principle and applies it first to the error layer —
the clearest instance — and, in the same cycle, to the other richest server-only
clusters (`auth`, `viewer`, `posts` helpers).

## The three-tier error model

There are three error tiers; only the wire tier is genuinely `web`. The
relationship between them is **conversion, not containment** — no tier holds
another; each is _converted into_ the next along a one-way pipeline
`T1 → T2 → T3`.

| Tier | Type                                         | Purpose                                                                                                         | Home (target)                                |
| ---- | -------------------------------------------- | --------------------------------------------------------------------------------------------------------------- | -------------------------------------------- |
| T1   | `AudienceError`, `UserAuthError`, … (16)     | typed, discrete domain-operation failures; callers match on them (ADR-0017 §1)                                  | `storage` / `common` (unchanged)             |
| T2   | `InternalError` + `ErrorKind` + `ErrorClass` | the **operator-side carrier**; never crosses the wire; classifies, logs, and emits a metric                     | **`host`** (new module — moves out of `web`) |
| T3   | `WebError`                                   | the **wire/public** type; serialized, crosses the network, deserialized on the wasm client; masked message only | `web` (stays)                                |

```
   in the server-fn body            at the boundary (boundary! / server_boundary)
   ─────────────────────            ───────────────────────────────────────────────
   T1 ──.into()/?──▶ T2             T2 ──log+metric──▶ project ──▶ T3 ──▶ wire ──▶ UI
   AudienceError     InternalError  InternalError                 WebError
   (at each fallible call)          (once, centrally, on Err)
```

- **T1 → T2** happens **inside the server-fn body**, at each fallible call, via
  `?` + `From<DomainError> for InternalError`. `boundary!` does _not_ do this
  conversion.
- **T2 → T3** happens **once, in `boundary!`/`server_boundary`, on the `Err`
  path**, and in this order: first **log the full T2** (source chain, context,
  at the class-derived level) and **emit the metric**, _then_ **project** T2
  down to a masked T3. All operator detail is spent on logs/metrics before it is
  discarded; the wire gets only the masked projection. T3 is a **lossy
  projection** of T2, never a wrapper around it.

**Why the tiers don't collapse** (the ADR elaborates this as its lead — see "ADR
plan"):

- **T2↔T3 is a security boundary made structural.** T3 crosses the wire to an
  untrusted client, so it must be serializable _and_ client-safe; T2 must retain
  the full operator payload (typed source chain, DB text, context,
  classification) that must never reach a client. One type cannot be both —
  merged, it either leaks operator detail or drops the cause before it can be
  logged. The split makes masking a _type_ boundary: the operator payload is
  structurally absent from the type that crosses the wire, so leakage is
  impossible by construction, not by discipline.
- **T1↔T2 is typed-discrete-cause vs uniform-carrier.** T1 keeps each
  operation's discrete failures typed for exhaustive mapping (`NotFound→404`,
  `Conflict→409`); T2 is the single uniform thing the boundary classifies, logs,
  and projects. Collapsing either boundary forfeits a specific guarantee
  (no-leak; exhaustive compiler-checked mapping; preserved-cause-for-triage).

## Design decisions (resolved in the interview)

### D1 — T2 carrier moves to `host`, decoupled from `WebError`

`InternalError`, `ErrorKind`, `ErrorClass`, and the boundary log/metric
machinery move from `web/src/error.rs` into a **new `host` module**
(`host/src/error.rs`). The carrier **drops the `public: WebError` field** — its
only web tie. In its place it carries a masked **`public_message: String`**
alongside the existing `kind` (category), `class` (severity), `context`
(operator k/v), and `source: Option<anyhow::Error>`.

Consequence: the carrier can no longer be constructed _from_ a `WebError`
(today's `From<WebError> for InternalError` + `kind_class_for` round-trip is
deleted). Construction is always via the typed constructors (`not_found`,
`storage`, …) or `From<DomainError>` — the direction we want; nothing mints a
`WebError` server-side to wrap it back up.

**Constructor rework (no `WebError` parameter survives).** Today several
constructors delegate through `From<WebError>`/`kind_class_for`, and
`masked(public: WebError, operator_message)` takes a whole `WebError`.
Post-decoupling they set their fields directly: the masking constructors
(`storage`/`server`/`external`) and `masked` take
`(kind, class, public_message, source)` — **no constructor on the carrier takes
a `WebError`**. This is what makes the D2 projection _total by construction_
rather than by convention: the carrier can only ever hold one of the seven
`ErrorKind`s, so `WebError::ServerFunction` (produced solely by
`FromServerFnError` on the client-deserialize path) is structurally unreachable
from it. The delegating client constructors
(`not_found`/`validation`/`conflict`) and `unauthorized`/`server_message`
likewise set `(kind, class, public_message)` directly, each storing the _exact_
current public string so A5/A7 hold byte-for-byte.
`not_found_error`/`private_post_not_found_error` (D5) rebuild on this reworked
`masked`. The `public()` accessor is deleted with the field; its callers (D5's
three `.public()` sites) move to `kind()`.

### D2 — `web` keeps `WebError` and owns the projection

`WebError` (wire type, `FromServerFnError` + `JsonEncoding`) stays in `web`.
`web` gains one small **total** projection,
`fn project(kind: ErrorKind, public_message: &str) -> WebError`:

| `ErrorKind`  | → `WebError`                                            |
| ------------ | ------------------------------------------------------- |
| `Auth`       | `Unauthorized` (message dropped — variant carries none) |
| `NotFound`   | `NotFound { message }`                                  |
| `Validation` | `Validation { message }`                                |
| `Conflict`   | `Conflict { message }`                                  |
| `Storage`    | `Storage { message }`                                   |
| `Internal`   | `Server { message }`                                    |
| `External`   | `Server { message }`                                    |

`WebError::ServerFunction` is leptos-internal (produced only by
`FromServerFnError` when the client fails to deserialize) and is unreachable
from the carrier, so the projection is total. The projection must reproduce
today's `into_public()` output byte-for-byte so the pinned JSON-encoding and
no-leak tests still hold.

### D3 — `boundary!` splits: owner-pinning stays in `web`; log+metric are `host` carrier methods

`server_boundary` currently does two things: (a) wraps the future in a
`ScopedFuture` and holds the leptos reactive-ownership **ancestry** strong
across awaits (ADR-0016 addenda #89/#124/#138 — irreducibly leptos), and (b) on
`Err`, logs + emits a metric + projects to public. The split:

- **Stays in `web`:** the owner-pinning wrapper (`ScopedFuture`,
  `owner_ancestry_strong`, `server_resource`) and the final `project(...)` call.
  The `boundary!` macro keeps its signature and call sites unchanged.
- **Moves to `host` (carrier methods):** `error.log_boundary_failure(server_fn)`
  (the structured `tracing` event over
  `kind`/`class`/`context`/`source`/`public_message`) and the
  `common::metrics::error(kind.as_metric_str(), class.as_metric_str())` emission
  — no leptos dependency. `common::metrics::error` keeps its `&'static str`
  signature (no change).

`web`'s `server_boundary` becomes: pin owners → run body → on `Err`, call
`err.log_boundary_failure(name)` (host) then return
`project(err.kind(), err.public_message())`.

### D4 — vertical mappers collapse into `From` impls in `storage`

Per-vertical `map_*_error` free fns (and equivalent inline
`.map_err(InternalError::…)` lifts) become
`impl From<DomainError> for InternalError`, so each call site is `err.into()` /
`?`. Placement follows the orphan rule:

- Source type local to `storage` (e.g. `AudienceError`, `PerformCreationError`)
  → the `From` impl lives in **`storage`** (which now depends on `host` to name
  `InternalError`).
- Source type local to `common` (e.g. `MailError`) → the impl lives in
  **`host`** (both the carrier and, via `host`→`common`, the source type are
  visible there).
- Web-local source types (e.g. `AuthRejection`) are addressed by D5, not by a
  bare `From`.

### D5 — broader push-down of `auth` / `viewer` / `posts` server-only code

The three richest non-error server-only clusters are pushed down in this cycle,
applying the thin-shell test **per item**: pure core moves down; a thin leptos
adapter stays. Each item is MOVE (pure → `host`/`storage`/`common`), SPLIT (pure
core down + named leptos adapter stays in `web`), or STAYS (irreducibly leptos).
Full table in "Push-down inventory" below; three cross-cutting rules govern it:

- **Wire types stay in `web` (the wasm-reachability rule).** Anything a
  `#[server]` fn _returns_ is a server-fn wire type the wasm client
  deserializes, so it **cannot** live in `host` (host never compiles to wasm).
  `PostResponse` (`posts/mod.rs`) and `TimelinePostSummary` (`posts/listing.rs`)
  are such types — they, and their `record → DTO` projection builders
  (`post_response`, `timeline_post_summary`), **stay in `web`**. This is the
  Ok-side analogue of the error projection staying in `web` (D2): `web` owns the
  whole server-fn wire surface — both the `Ok` DTOs and the `Err` `WebError`
  plus their two projections. What moves down is the _effectful_ work that feeds
  the projection (fetching the record, pagination, tag orchestration), not the
  projection itself. (This corrects the classification pass, which flagged the
  DTO builders as movable without accounting for the wasm constraint.)
- **Every error lift becomes typed — `From` where the lift is canonical, a
  source-preserving constructor where the public message is site-specific.**
  storage-local sources → `From` in `storage` (D4, incl. `TaggingError` and the
  `PostFormat` parse error); common-local (`InvalidSlug`, `Username`/`Tag`
  parse, `MailError`) and external `sqlx::Error` → `From` in **`host`** (which
  takes `sqlx` + `http` as external infra deps, D6). `From<sqlx::Error>` is
  **behavior-preserving** — exactly today's `InternalError::storage(e)` (kind
  `Storage`, source preserved for the boundary downcast), reachable via `?`; no
  classification change this cycle (SQLSTATE-aware refinement is a deliberate
  non-goal). `TaggingError` maps to its _current_ wire classification (kind
  `Internal` → public `Server`), de-stringified. **`chrono` parse failures do
  _not_ get a blanket `From`**: their public messages are site-specific
  (`"invalid publish_at: …"`, `"invalid cursor_created_at"`), which one
  `From<chrono::ParseError>` would flatten — a wire/UX regression. They route
  through the reworked
  **`masked(Validation, Client, <site public message>, anyhow::Error::new(e))`**
  (the `anyhow` `source` chain _is_ the `.context()` mechanism, on the operator
  side, while the site message stays on the wire) — no new constructor, no
  chrono `From`, and no `chrono` dep on `host` (the `anyhow::Error::new` is at
  the call site). This kills the lossy `.to_string()` (A19) and keeps the
  per-site message. The `impl Error`-taking constructors otherwise remain only
  for ADR-0017's heterogeneous `Box<dyn Error>` sites. "Behavior-preserving"
  overall means the **wire projection is unchanged** (pinned byte-for-byte by
  A5/A7); only the operator-side source becomes typed.
- **Destination is constrained by _types touched_, not just "no leptos".** A
  pure item that takes or returns a `storage` type (a `&dyn *Storage`,
  `PostRecord`, a storage error enum) **cannot** live in `host` (floor
  invariant, D6) — its home is `storage`, which depends on `host` and so can
  both see the storage type and return `host::InternalError`. This governs the
  effectful posts/viewer orchestration below.
- **The `.public()` accessor is removed with the field; its three branch sites
  are rewritten and stay in `web`.** `classify_current_user` **and**
  `backup/mod.rs` (two `#[server]` bodies) currently branch on
  `error.public() == WebError::Unauthorized`; all three rewrite to
  `error.kind() == ErrorKind::Auth`. They operate on web-local values, so they
  **stay in `web`** — the change only sheds the `WebError` dependency, not
  relocation.

### D6 — `host` layering: the floor invariant (extends ADR-0058)

This cycle promotes `host` from a near-leaf utility crate (ADR-0058: dependents
`server`, `test-support`) to the host **floor**: it gains `common` + `anyhow` +
`tracing` as deps and `storage` + `web` as dependents. The load-bearing
invariant, recorded in this cycle's ADR:

> **`host` depends on no _workspace_ crate except `common`.** Never
> `storage`/`web`/`server` — only workspace-crate deps can cycle or recreate the
> `AppState` omnibus. It **may** depend on external _infrastructure_ crates the
> carrier actually needs (`anyhow`, `tracing`, `sqlx`, `http`). The dividing
> line: **`host` knows raw infrastructure types** (driver errors like
> `sqlx::Error`, HTTP headers) for classification/parsing — **but not our
> domain/storage abstractions** (`PostStorage`, `AudienceError`, …), which stay
> above it. That line is exactly why `fetch_post_record` (uses
> `&dyn PostStorage`) homes in `storage` while `From<sqlx::Error>` (a raw driver
> type) homes in `host`. `server` remains the host _ceiling_ (composition root);
> `host` is the _floor_.

Target graph (workspace crates): `common → host → storage → web → server` (with
`web`/`server` also depending on `host`; `web`→`host`/`web`→`storage` are
`server`-feature-gated). `host` additionally depends on external
`sqlx`/`http`/`anyhow`/`tracing` (not `chrono` — its parse errors reach the
carrier via a generic constructor monomorphized at the call site). Acyclic.

### D7 — naming

`InternalError` reads oddly once it lives in `host` ("internal to what?"). The
rename (candidates: keep `InternalError`, `host::BoundaryError`,
`host::ServerError`) is decided in the ADR. Non-blocking for the design.

## Sequencing & blocker

- **Blocked by #227 / PR #335** (native GitHub dependency recorded). `host` does
  not exist on `main`; PR #335 introduces it and will be force-pushed before it
  merges.
- **Develop now; keep the `host`-crate footprint additive.** The cycle edits
  several crates (`web`, `storage`, `common`, plus the `host`/`storage` dep
  lines) — but _inside the `host` crate_ it touches **only a new
  `host/src/error.rs` module**, a `pub mod error;` line, and the additive deps
  that module needs (`anyhow`, `tracing`, `common`, `sqlx`, `http`). It does
  **not** recreate or restructure #335's capture module — the branch never
  creates `capture.rs`, so the only collision surface with #335's force-pushed
  `host` is the `lib.rs` module list and `Cargo.toml` dep list, both additive.
- **Ship after #335 merges.** Post-merge, rebase 334 onto the new `main`; the
  rebase should reduce to re-adding `pub mod error;` to the merged `lib.rs` and
  merging dep lines into the merged `Cargo.toml`. PR/merge of 334 waits for
  #335.

## Scope

**In scope this cycle:** the ADR; D1–D7; the carrier relocation + decoupling;
the `web` projection; the `boundary!` split; all vertical mapper → `From`
conversions; and the `auth`/`viewer`/`posts` push-down (D5).

**Out of scope:** the UI co-location (`pages/` → co-located feature modules) —
that is the #303/#314 axis. #314 (audiences convergence) **rebases on top of**
this work. This cycle does not move `#[component]` UI or touch `pages/`.

## Acceptance criteria (observable)

Stated so a later conformance review can tell delivered from not.

**Carrier relocation & decoupling (D1):**

- A1. `InternalError`, `ErrorKind`, `ErrorClass` are defined in `host` (module
  `host/src/error.rs`), not in `web/src/error.rs`.
- A2. `InternalError` has **no** field of type `WebError` (nor any `web`-crate
  type); `grep` for `WebError` in `host/` finds nothing.
- A3. `host/Cargo.toml` depends on **no workspace crate except `common`** — no
  `storage`, `web`, `server`, `leptos`, or `axum`. It may depend on external
  infra crates (`anyhow`, `tracing`, `sqlx`, `http`); **not** `chrono` (its
  parse errors reach the carrier via a generic constructor). (Floor invariant,
  D6.)
- A4. `host` names no _workspace-`storage`_ abstraction (no `use storage::…` in
  `host/src` — `PostStorage`, `AudienceError`, etc.). Naming the raw external
  `sqlx::Error` is expected and allowed.

**Wire type & projection (D2):**

- A5. `WebError` remains defined in `web` and still impls `FromServerFnError` +
  `JsonEncoding`; the wire JSON encoding test is unchanged and green.
- A6. `web` contains the total `ErrorKind → WebError` projection; the previous
  `From<WebError> for InternalError` and `kind_class_for` are gone.
- A7. The no-leak test (masked public error never contains source-chain text)
  still passes against the projected output.

**Boundary split (D3):**

- A8. `boundary!` macro signature and all `#[server]` call sites are unchanged.
- A9. Owner-pinning (`ScopedFuture` ancestry hold, `server_resource`) still
  lives in `web`; the `owner_lifetime` tests are unchanged and green.
- A10. `log_boundary_failure` + the metric emission are `host` carrier methods
  (no leptos import in that path); on `Err`, `web`'s `server_boundary` calls
  them **before** projecting.

**Mapper collapse (D4):**

- A11. `map_audience_error` (and the other converted vertical mappers) are gone;
  call sites use `err.into()` / `?`.
  `impl From<AudienceError> for InternalError` lives in `storage`.

**Push-down (D5):**

- A12. Every item classified MOVE in the inventory is gone from `web/src` and
  lives at its stated destination — the storage-coupled orchestration items
  (`local_channel_id`, `apply_post_tag_diff`, `fetch_post_record`,
  `find_draft_by_permalink_for_user`, `list_by_tag_rows`) in **`storage`**, the
  pure ones in `host`/`common` — and its logic is exercised by a host-side test
  at the new home (win realized, not just relocated).
- A13. Each SPLIT item's retained `web` adapter body consists only of the named
  leptos/axum calls (from the "retained adapter surface" list) plus a single
  call into the pushed-down core; the pure logic named in the inventory is not
  duplicated in `web`.
- A14. `PostResponse`, `TimelinePostSummary`, and their builders remain in
  `web`; no server-fn return type is defined in `host`. (`host`'s freedom from
  leptos/web/storage deps is A3/A4.)
- A15. The named per-vertical mappers (D4 table) are gone; their conversions are
  `impl From<…>` in `storage` (or `web` for `auth_rejection_error`); call sites
  use `?`/`.into()`.

**Cross-cutting:**

- A16. No `#[cfg(feature = "server")]` item remains in `web/src` that has no
  leptos/wasm/wire tie — checked against the D5 inventory, which is **closed**:
  it now includes `TaggingError`, the `PostFormat` parse error, and the two
  `backup/mod.rs` `.public()` sites (A20).
- A17. `cargo xtask validate` green (static + clippy incl. wasm-target +
  coverage + full e2e matrix), after the #335 rebase.

**Error-handling completeness (D4, per your "handle it this cycle"):**

- A18. Every typed error source has a typed conversion at its correct home:
  storage-local (the D4 mappers **plus** `TaggingError` and the `PostFormat`
  parse error) → `From` in `storage`; common-local (`InvalidSlug`,
  `Username`/`Tag` parse, `MailError`) and external `sqlx::Error` → `From` in
  `host`; `chrono` parse (site-specific messages) → the existing
  `masked(Validation, Client, <site message>, anyhow::Error::new(e))` (no
  blanket `From`, no new constructor, so `host` needs no `chrono` dep). Call
  sites use `?`/`.into()` or `masked`. `host` depends on `sqlx`/`http`, not the
  `storage`/`web` workspace crates (A3/A4).
- A19. No lossy `.map_err(|e| …validation(e.to_string()))` (or equivalent
  `.to_string()` of a typed source) remains in `web/src`; every typed source
  reaches the carrier with its chain intact (downcastable, ADR-0017 §3).
- A20. The `public: WebError` field **and** the `.public()` accessor are gone;
  every site that branched on `.public() == WebError::Unauthorized` —
  `classify_current_user` and the two `backup/mod.rs` `#[server]` bodies —
  branches on `kind() == ErrorKind::Auth` and stays in `web`. (Byte-for-byte
  wire output preserved: A5/A7.)

## Push-down inventory (D5)

Classification of every server-only item in the three clusters. Anchors are
current (pre-refactor) `web/src` locations.

### Cluster A — `auth/server.rs`

| Item                                                          | Class               | Destination / retained adapter                                                                                                                                      |
| ------------------------------------------------------------- | ------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `parse_basic_auth`                                            | MOVE                | → `common` (pure `&str` → `(user,pass)` base64)                                                                                                                     |
| `resolve_credential` (+ private `Credential`)                 | MOVE                | → `host` (pure parse over `http::HeaderMap`; adds light `http` dep to host)                                                                                         |
| `session_outcome`, `login_outcome`                            | MOVE                | → `storage` (pure `SessionAuthError`/`UserAuthError` → `common::metrics` outcome; sit by the source enums)                                                          |
| `CookieSettings` (struct)                                     | MOVE                | → `host` (pure config data; read via context by the cookie adapters)                                                                                                |
| `AuthUser` (struct) + `impl FromRequestParts`                 | STAYS               | axum's `FromRequestParts` is a foreign trait ⇒ the type must be `web`-local to impl it (orphan rule); the impl also reads `Arc<dyn SessionStorage>` from extensions |
| `verify_basic_username`                                       | SPLIT               | pure `Username` comparison → down; thin `AuthRejection`-typed wrapper STAYS (source result type is web-local)                                                       |
| `set_session_cookie`, `clear_session_cookie`                  | SPLIT               | pure header-string builder (`session=…; HttpOnly; …`) → `host`; adapter STAYS: `use_context::<CookieSettings/ResponseOptions>` + `insert_header(SET_COOKIE)`        |
| `classify_current_user`                                       | STAYS               | rewritten to branch on `kind()==Auth` (drops `WebError`), but operates on web-local `AuthUser`                                                                      |
| `impl FromRequestParts for AuthUser`                          | STAYS               | axum extractor over `Parts` / `extensions`                                                                                                                          |
| `require_auth_with_parts`                                     | STAYS               | drives the extractor over `http::request::Parts`                                                                                                                    |
| `require_auth`                                                | SPLIT               | body is one `use_context::<Parts>()` call (adapter STAYS) → `require_auth_with_parts`                                                                               |
| `AuthRejection` (enum) + `impl IntoResponse`                  | STAYS               | web-local HTTP-extraction error → `StatusCode`                                                                                                                      |
| `auth_rejection_error`                                        | STAYS               | source `AuthRejection` is web-local → `From` stays in `web`                                                                                                         |
| `register_open_error`, `register_invite_error`, `login_error` | → From in `storage` | sources `CreateUserError`/`RegisterWithInviteError`/`UserAuthError` (all storage-local)                                                                             |

### Cluster B — `viewer.rs`

| Item                                               | Class | Destination / retained adapter                                                                                                                                   |
| -------------------------------------------------- | ----- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `account_viewer`, `viewer_user_id`                 | MOVE  | → `common` (pure `ViewerIdentity` projections; type already in `common`)                                                                                         |
| `local_channel_id` (+ `LOCAL_CHANNEL_ID` OnceLock) | MOVE  | → **`storage`** (takes `&dyn SubscriptionStorage` ⇒ cannot be in `host`; effectful memoized call, static moves too)                                              |
| `viewer_identity`                                  | SPLIT | pure/effectful halves already factored out (move down); adapter STAYS: `leptos_axum::extract::<AuthUser>()` + `expect_context::<Arc<dyn SubscriptionStorage>>()` |

### Cluster C — `posts/server.rs`

| Item                                                                                      | Class               | Destination / retained adapter                                                                                                           |
| ----------------------------------------------------------------------------------------- | ------------------- | ---------------------------------------------------------------------------------------------------------------------------------------- |
| `apply_post_tag_diff`, `fetch_post_record`, `find_draft_by_permalink_for_user`            | MOVE                | → **`storage`** (take `&dyn PostStorage`/`PermalinkDate` ⇒ cannot be in `host`; effectful orchestration returning `host::InternalError`) |
| `list_by_tag_rows`                                                                        | MOVE                | → **`storage`** (returns `Result<Vec<PostRecord>, ListByTagError>`; business rule `TagNotFound→empty`, so a helper, not a `From`)        |
| `to_post_cursor`, `parse_post_cursor`                                                     | MOVE                | → `storage` (`PostCursor` is storage-local; not a wire type)                                                                             |
| `post_response`, `timeline_post_summary` + the `PostResponse`/`TimelinePostSummary` types | **STAYS**           | server-fn **wire types** + their Ok-projection (wasm-reachability rule)                                                                  |
| `set_not_found_status`                                                                    | STAYS               | `use_context::<ResponseOptions>()` + `set_status(404)` — the shared status adapter                                                       |
| `not_found_error`, `private_post_not_found_error`                                         | SPLIT               | adapter = `set_not_found_status()` (STAYS); carrier construction moves with the carrier                                                  |
| `perform_update_error`, `perform_creation_error`                                          | → From in `storage` | sources `PerformUpdateError`/`PerformCreationError` (storage-local)                                                                      |

### Mapper → `From` conversions (D4)

| Mapper                                                   | source type                              | source crate | `From` impl home                                                      |
| -------------------------------------------------------- | ---------------------------------------- | ------------ | --------------------------------------------------------------------- |
| `map_audience_error`                                     | `AudienceError`                          | storage      | **storage**                                                           |
| `register_open_error`                                    | `CreateUserError`                        | storage      | **storage**                                                           |
| `register_invite_error`                                  | `RegisterWithInviteError`                | storage      | **storage**                                                           |
| `login_error`                                            | `UserAuthError`                          | storage      | **storage**                                                           |
| `perform_update_error`                                   | `PerformUpdateError`                     | storage      | **storage**                                                           |
| `perform_creation_error`                                 | `PerformCreationError`                   | storage      | **storage**                                                           |
| inline `match` at `posts/mod.rs:469,644`                 | `PerformUpdateError` / `UpdatePostError` | storage      | **storage**                                                           |
| `tag_post`/`untag_post` lift (in `apply_post_tag_diff`)  | `TaggingError`                           | storage      | **storage** (maps to kind `Internal`→public `Server`, de-stringified) |
| `format.parse::<PostFormat>()` lift (`posts/mod.rs:243`) | `PostFormat` parse error                 | storage      | **storage**                                                           |
| `auth_rejection_error`                                   | `AuthRejection`                          | web-local    | **web** (stays)                                                       |

Two `#[server]` bodies in `backup/mod.rs` (~:28, :62) branch on
`error.public() == WebError::Unauthorized`; like `classify_current_user` they
rewrite to `kind() == ErrorKind::Auth` and **stay in `web`** (the `.public()`
accessor is deleted with the field).

Typed sources with a _canonical_ lift get a `From`: storage-local (the D4
mappers **plus** `TaggingError` and the `PostFormat` parse error) → `storage`;
common-local (`InvalidSlug`, `Username`/`Tag` parse, `MailError`) and external
`sqlx::Error` → `host` (which takes `sqlx` + `http` as infra deps, D6).
`From<sqlx::Error>` is behavior-preserving (== today's
`InternalError::storage(e)`); `From<TaggingError>` preserves its current wire
class (kind `Internal` → public `Server`), de-stringified. `chrono` parse
failures route through the existing
`masked(Validation, Client, <site message>, anyhow::Error::new(e))`
(site-specific messages; no blanket `From`, no new constructor, so `host` needs
no `chrono` dep). Every lossy `.to_string()` of a typed source is eliminated;
the `impl Error`-taking constructors otherwise survive only for ADR-0017
heterogeneous `Box<dyn Error>` sites.

### Retained `web` adapter surface (the legitimate thin shell)

`web` keeps: (1) the axum extractor + `Parts`/`HeaderMap` request surface
(`AuthUser::from_request_parts`, `require_auth_with_parts`); (2) `AuthRejection`
→ `StatusCode` response mapping; (3) the three
`ResponseOptions`/`CookieSettings` side-effect adapters (`set_session_cookie`,
`clear_session_cookie`, `set_not_found_status`); (4) leptos context extraction
in `require_auth`/`viewer_identity`; (5) the `FromServerFnError` wire glue; (6)
the server-fn wire types (`WebError`, `PostResponse`, `TimelinePostSummary`) and
their Ok/Err projections; (7) the `#[server]` fns themselves and all client/wasm
`pages/`.

## ADR plan

Author a numberless draft (`jaunder-adr`) → promoted to **ADR-0059** at ship.
References ADR-0017 (picks up its "forthcoming structured carrier" thread),
ADR-0016 (DI / anti-omnibus), and ADR-0058 (host layering — which it extends).

**Required lead content — the T1/T2/T3 _why_, written so no one re-derives it.**
The ADR must make the reason the three tiers exist and stay separate impossible
to miss. State it as two boundaries, each justified by a guarantee that
collapsing it would forfeit:

- **T2 ↔ T3 — a security boundary made structural.** T3 (`WebError`) crosses
  the network to an _untrusted client_, so it must be
  serializable/wasm-compatible **and** carry only client-safe data. T2
  (`InternalError`) must retain the full operator payload — typed source chain
  (DB/driver text, SQLSTATE), operator context, classification — none of which
  may reach a client (ADR-0017's leak failure mode). One type cannot satisfy
  both: merged, it either leaks operator detail over the wire, or drops the
  cause before it can be logged. Splitting makes the masking boundary a **type**
  boundary — T2→T3 is a lossy one-way projection, so the operator payload is
  _structurally absent_ from the type that crosses the wire; leakage is
  impossible **by construction, not by discipline**. (Homes follow from the same
  fact: T3 lives with leptos/wasm in `web`; T2 is target-agnostic host machinery
  in `host`.)
- **T1 ↔ T2 — typed discrete cause vs uniform carrier.** T1 models the discrete
  _expected_ failures of one operation, kept typed so callers map each
  exhaustively (`NotFound→404`, `Conflict→409`; invalid states unrepresentable,
  ADR-0017 §1). T2 is the single uniform thing the boundary classifies, logs,
  and projects. Collapsing T1→T3 directly loses the typed cause
  - classification before logging (T3 cannot carry them) — reintroducing
    ADR-0017's lossy-stringification failure. Collapsing T1 into T2 from the
    start loses compiler-checked exhaustive mapping of discrete failures.

Then the ADR records: the **thin web shell** principle as a durable invariant (à
la ADR-0016); the conversion-not-containment pipeline; the carrier in `host`
decoupled from the wire type; the `host` **floor invariant** (extends ADR-0058;
adds `storage`/`web` dependents); and the naming decision (D7).

## Risks / open questions

- **R1.** Rebase churn on `host/{Cargo.toml,lib.rs}` after #335's force-push +
  merge — mitigated by the additive-only footprint (D-sequencing).
- **R2.** D5 overlaps #303/#314's per-vertical convergence on `auth`/`posts`.
  Mitigation: 334 lands first; #314 rebases on top (per issue #334 note). This
  cycle touches only server-only _logic_ push-down, not UI co-location.
- **R3.** Some `auth` guards may be more leptos-entangled than a clean SPLIT
  allows (cookies, `Parts`, `ResponseOptions`) — the inventory records exactly
  what stays as the thin adapter; if a candidate is STAYS, that is a recorded
  outcome, not a failure.
