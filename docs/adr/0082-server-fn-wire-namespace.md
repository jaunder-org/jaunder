# ADR-0082: Server-fn wire URLs are `/api/<vertical>/<op>`, derived and gate-written

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

`endpoint` stays **explicitly pinned** on every fn — never omitted — so the URL
is stable, readable, and independent of where the repository is checked out.

The literal is **derived, not authored**. The `server-fn-endpoint` gate computes
the expected value and, under `Mode::Fix` (`cargo xtask check`), writes it;
`Mode::Check` (`cargo xtask validate`, and CI) verifies without mutating. This
is the same contract `fmt` has, and the same one ADR-0011's `server-fn-tracing`
gate already applies to span names — `endpoint` is now the second derived
literal on a server fn, and both are the gate's to maintain rather than the
author's to keep in sync by hand.

Unlike the span-name gate, a **missing** `endpoint` is reported, never
synthesized. Omitting it is a decision with the hash consequence above; the gate
refuses to make that decision on an author's behalf.

Rust callers do not hardcode these URLs. `server/tests/**` names
`<web::<vertical>::<Type> as ServerFn>::PATH` — a public associated const on the
generated type (`server_fn-0.8.12/src/lib.rs:220`) — so a Rust call site cannot
drift from the attribute at all.

## Consequences

- **The wire is a private surface, and this confirms it.** `/api/*` is the CSR
  client's own protocol. The public, stable API is AtomPub, which is untouched.
  Renaming a server fn is therefore a wire-visible change that costs nothing
  externally.
- **Endpoint uniqueness is entirely gate-enforced.** A pinned `endpoint`
  suppresses the disambiguating hash, so nothing else prevents a collision. The
  gate fails on any two fns deriving the same endpoint. That check is defence in
  depth rather than an independent guarantee: because the value is derived from
  `(vertical, ident)`, a collision requires a duplicate `(vertical, ident)`,
  which ADR-0066's registrar gate already hard-fails.
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
- **A third derived literal remains unguarded.** Each `#[server]` body wraps
  itself in `boundary!("<fn ident>")`, which becomes the ADR-0011 structured-log
  field naming the failing server fn. Nothing correlates it with the ident — not
  the compiler, not a gate — so #684 moved all 42 by hand.
  [#714](https://github.com/jaunder-org/jaunder/issues/714) proposes the gate;
  it needs `syn` traversal into the fn body rather than the shared
  attribute-rewrite machinery, which is why it is not part of this decision.
- **Adding a vertical costs nothing here.** The scheme is computed, so a new
  `web/src/<vertical>/` yields conforming endpoints with no registry to update.
  A `#[server]` fn placed directly under `web/src` has no vertical and is a hard
  error in all three server-fn gates — a deliberate tightening, since there is
  no honest name to derive.
