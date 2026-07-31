# ADR-0082: Server-fn wire URLs are `/api/<vertical>/<op>`, derived and macro-written

- Status: proposed
- Date: 2026-07-29
- Issue: [#684](https://github.com/jaunder-org/jaunder/issues/684)

## Context

`server/src/lib.rs` mounts every `#[server]` fn under a single wildcard route,
`"/api/{*fn_name}"`. So the 55 server-fn URLs live in **one flat global
namespace**, and uniqueness across it is a property nothing in the type system
maintains.

Until #684 the namespace was kept unique by accident: each `endpoint` was
`"/" + the fn ident`, and the idents carried a vertical noun — `create_post`,
`create_audience`, `create_invite`. When #684 dropped those nouns (the module
path `web/src/<vertical>/` already states them), all three verticals' `create`
wanted `/api/create`. **The vertical noun turned out to be load-bearing in the
endpoint exactly where it was vestigial in the ident** — the inverse of the
rename's premise, and the reason the endpoints could not simply follow the fns.

Dropping `endpoint` entirely and taking `server_fn`'s default was considered and
rejected. Without it, `server_fn_macro-0.8.10/src/lib.rs:483-546` derives the
URL as `prefix + "/" + fn_name + hash`, where the hash (`:515-521`) is
`xxh64(CARGO_MANIFEST_DIR + ":" + module_path!())`. That is an **absolute
path**, so the URL differs between checkouts of the same commit. Harmless for a
single build — client and server compile together and agree — but it makes the
wire unnameable in documentation, unhardcodable in a test, and different in a
worktree than in the main checkout. (`SERVER_FN_OVERRIDE_KEY` stabilises the
hash, but the best it yields is `/api/create14229099282181147008`.)

## Decision

**Every `#[server]` fn's `endpoint` is `/<vertical>/<fn ident>`**, giving a wire
URL of `/api/<vertical>/<op>`. The vertical is the first path segment under
`web/src`.

Because the URL _is_ the fn ident, the ident's naming rule is a wire rule: **the
vertical's own noun is dropped** (the path already states it —
`audiences::create`, not `audiences::create_audience`) and **the ident is
verb-led**, with getters taking `get_` and boolean predicates `is_`. That was
already true of 49 of the 55 fns before this decision; stating it closed the six
exceptions rather than establishing anything new.

The rule reaches further than the wire, because the `#[server]` macro derives a
**public struct** from the ident — `PascalCase(fn ident)`, holding the fn's
parameters (`server_fn_macro-0.8.10/src/lib.rs:394-435`). That struct occupies
the vertical's namespace whether or not anyone intends it to, so a poorly-chosen
ident can squat on a domain type's name. It did: `posts::audience_selection`
generated an `AudienceSelection` holding just a `post_id` — a _request for_ a
selection, not a selection — colliding with
`common::visibility::AudienceSelection`. The verb rule resolves that class by
construction, since `Get…`/`Is…` names do not collide with domain nouns.

The literal is **derived, not authored**, so the URL is stable, readable, and
independent of where the repository is checked out.

_Amended by #714._ The derivation moved from a gate to a macro. This section
originally read that `endpoint` "stays **explicitly pinned** on every fn — never
omitted"; that "the `server-fn-endpoint` gate computes the expected value and,
under `Mode::Fix` (`cargo xtask check`), writes it" while `Mode::Check` verifies
it — "the same contract `fmt` has"; and that "a **missing** `endpoint` is
reported, never synthesized". None of that machinery exists any more.
`#[macros::server]` (`macros/src/server_fn.rs`) derives `/<vertical>/<fn ident>`
from the fn's file path and identifier and emits
`#[::leptos::server(endpoint = "…")]` in its expansion; the `server-fn-endpoint`
gate is deleted. The value and the scheme are unchanged — only who writes them.
Omitting `endpoint` is no longer a decision an author can make, so the hash
consequence above is unreachable **by construction** rather than refused by a
gate. The derivation depends on a placement rule #714 adds (ADR-0070): every
`#[server]` fn lives in `web/src/<vertical>/api.rs`, which the macro hard-errors
on.

Rust callers do not hardcode these URLs. `server/tests/**` names
`<web::<vertical>::<Type> as ServerFn>::PATH` — a public associated const on the
generated type (`server_fn-0.8.12/src/lib.rs:220`) — so a Rust call site cannot
drift from the attribute at all.

### Rejected alternatives (recorded by #714)

`server_fn_macro` exposes two compile-time knobs that reshape the default URL.
Neither was named here originally; both were considered and rejected.

- **`DISABLE_SERVER_FN_HASH`** (`server_fn_macro-0.8.10/src/lib.rs:510`) drops
  the `xxh64` suffix, so a bare `#[server]` yields `/api/<fn ident>` with no
  attribute written anywhere — the variant proposed by
  [#698](https://github.com/jaunder-org/jaunder/issues/698). It is **unsafe
  after #684**: with the vertical noun shed from the idents, all three
  verticals' `create` want `/api/create`. #698 named #426's duplicate-leaf-name
  guard as its precondition, but #684 re-keyed that guard to `(vertical, leaf)`
  (ADR-0066), and a `(vertical, leaf)` key by design does **not** fail on the
  same leaf in different verticals — precisely the collision this variant
  produces. It is also an `option_env!`, so the wire would depend on a build
  environment variable rather than on the source.
- **`SERVER_FN_MOD_PATH`** (`:497-508`) prepends `module_path!()` with `::`
  rewritten to `/`, giving `/api/web/posts/api/create…`. It is collision-proof
  by construction, but it changes all 55 URLs, puts the crate name and the
  private `api` module on the wire, and still stacks the hash unless
  `DISABLE_SERVER_FN_HASH` is set too — two coupled build-environment knobs
  where the decision above is one derivation rule. Module-path keying remains
  the recorded escape hatch if a vertical ever genuinely outgrows a single
  `api.rs`; taking it would be a deliberate URL change rather than a silent
  collision.

## Consequences

- **The wire is a private surface, and this confirms it.** `/api/*` is the CSR
  client's own protocol. The public, stable API is AtomPub, which is untouched.
  Renaming a server fn is therefore a wire-visible change that costs nothing
  externally.
- **Endpoint uniqueness rests on the placement rule, not on an endpoint gate.**
  A pinned `endpoint` suppresses the disambiguating hash, so nothing in the
  generated code prevents a collision. Three layers stand behind it: the #714
  **placement rule** (every `#[server]` fn lives in
  `web/src/<vertical>/api.rs`), which makes `(vertical, ident)` a primary key
  enforced by **rustc** — the vertical is unique because it is a directory, the
  ident because Rust forbids two items of one name in one module; ADR-0066's
  registrar gate, which hard-fails a duplicate `(vertical, leaf)`; and a runtime
  pairwise-distinctness assertion over every generated `ServerFn::PATH`
  (`server/tests/web/server_fn_wire.rs`), which reads the real types and so
  holds even if xtask's syn enumeration breaks.

  _Amended by #714._ This bullet originally read "**Endpoint uniqueness is
  entirely gate-enforced.** … The gate fails on any two fns deriving the same
  endpoint." That check died with `server_fn_endpoint_check.rs`. Losing it is a
  deliberate trade, not a cleanup: it was a real defence-in-depth layer, and it
  is given up because the placement rule turns the case it caught into a compile
  error instead of a gate failure. The price is that uniqueness now depends on
  the placement rule holding — relax it and `(vertical, ident)` goes lossy
  again, with the compiler unable to help, since a glob re-export lets one item
  silently shadow another so the duplicate compiles and the loser 404s (#358).

- **`/api/{*fn_name}` must keep matching multi-segment paths.** Dispatch reads
  the full request path (`leptos_axum-0.8.9/src/lib.rs:383-387`) and looks it up
  exactly, so nothing parses the captured segment — but the route must still
  match. `matchit-0.8.4`'s own executed doctest (`src/lib.rs:47-48`) asserts
  that a catch-all captures a multi-segment remainder.
  `server/tests/web/router.rs::multi_segment_server_fn_route_is_reachable` pins
  it, so an axum upgrade cannot silently 404 every server-fn route at once.
- **The Playwright suite is the one caller that can still drift.** No constant
  crosses the language boundary, so `end2end/tests/**` hardcodes these URLs —
  and matches some of them on the bare fn ident rather than the path. #684
  demonstrated the failure mode: after the wire moved, four specs failed with a
  30-second `waitForResponse` timeout rather than anything naming the cause. A
  TypeScript-side guard is
  [#712](https://github.com/jaunder-org/jaunder/issues/712).
- **The third derived literal was deleted rather than guarded.** This bullet
  originally recorded that each `#[server]` body wrapped itself in
  `boundary!("<fn ident>")` — the ADR-0011 structured-log field naming the
  failing server fn, correlated with the ident by nothing — and forecast
  [#714](https://github.com/jaunder-org/jaunder/issues/714) as a gate needing
  `syn` traversal into the fn body. That forecast is superseded: #714 removed
  the label instead of gating it. `#[macros::server]` wraps every body in
  `crate::error::server_boundary` unconditionally, so the boundary can no longer
  be omitted, and the label it carried is gone — the enclosing span already
  names the fn, and since #684 does so unambiguously. `boundary!` itself is
  deleted (see the ADR-0011 #714 addendum).
- **Adding a vertical costs nothing here.** The scheme is computed, so a new
  `web/src/<vertical>/` yields conforming endpoints with no registry to update.
  A `#[server]` fn placed directly under `web/src` has no vertical and is a hard
  error — a deliberate tightening, since there is no honest name to derive.
  _Amended by #714:_ that error was raised by all three server-fn gates; it is
  now raised by `#[macros::server]` at expansion, along with the rest of the
  placement rule.
- **The derived literals no longer appear anywhere in the tree, and the
  greppability argument is deliberately traded away.** Both the deleted endpoint
  gate and ADR-0011's span-name gate justified writing the derived value into
  the source precisely so an operator holding a URL or a span name could grep
  for the literal. After #714 neither string exists until macro expansion. The
  trade is accepted — a literal nobody writes cannot drift — but it is a real
  loss, and it costs most where a consumer hardcodes a URL and no compiler sees
  it: `end2end/tests/**`, already flagged above as the one caller that can still
  drift (#712). Grepping the wire now finds the TypeScript hardcode and nothing
  on the Rust side to correlate it against.
- **`cargo xtask check` no longer rewrites anything under `web/src`.** Both
  auto-fixing server-fn gates are gone — `server-fn-endpoint` with #714, and
  ADR-0011's span-name fix-mode with it — so `Mode::Fix` survives only for the
  formatting commands. The "same contract `fmt` has" that this decision invoked
  no longer applies to server fns at all.
