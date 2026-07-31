# ADR-0066: Guard the server-fn test registrar with an xtask check

- Status: accepted
- Date: 2026-07-13
- Issue: [#426](https://github.com/jaunder-org/jaunder/issues/426)

## Context

Integration and router tests can only route a `web` `#[server]` fn if its
generated type is named in a hand-maintained registrar
(`server_fn::axum::register_explicit::<web::…>()`). This exists because the test
binaries link `jaunder`/`web` as rlibs, and dead-code elimination drops each
`#[server]` macro's auto-registration (`inventory`-based) unless the type is
referenced explicitly. The production server keeps its registrations because it
_is_ the crate the macro expands in; the tests do not.

The hand list therefore rots silently: a new `#[server]` fn compiles and passes
its own crate's tests, but its route 404s in integration until someone adds it
by hand (this bit us in #358). At the time of this decision there were **two**
such lists — a complete one in `server/tests/helpers/mod.rs` and a 6-entry
subset in `server/src/lib.rs`'s `#[cfg(test)]` module — and the complete one was
already missing 10 real server fns.

Three approaches were considered:

- **A — guard the existing lists.** Keep both hand lists; add an `xtask` check
  that fails when a `web` `#[server]` fn is absent.
- **B — auto-register.** Emit each `#[server]` fn into a `linkme` distributed
  slice (via a wrapper attribute macro in the `macros` crate) that the test
  helper iterates, deleting the hand list entirely — "make illegal states
  unrepresentable."
- **C — consolidate, then guard.** Collapse the two lists into one, then guard
  the single list.

Constraint that shaped the choice: `server_fn` is a **dev-dependency** of
`server`, so a shared `pub fn register_all()` cannot live in `server/src`
non-test code without promoting `server_fn` to an optional dependency behind a
new `test-support` feature (plus a self dev-dependency to enable it for the
integration tests).

## Decision

Adopt **C + mandatory + a `syn`-based `xtask` gate**.

- **One registrar.**
  `server/tests/helpers/mod.rs::ensure_server_fns_registered()` is the sole
  list. The `server/src/lib.rs` subset is deleted and its registration-dependent
  router tests are relocated to an integration test that calls the shared
  helper. Relocation (rather than a shared `test-support` fn) is chosen because
  `server_fn` is dev-only: relocation reuses the existing integration idiom
  (`jaunder::create_router` + `ensure_server_fns_registered`) with zero
  Cargo/feature surface.
- **Mandatory.** Every `web` `#[server]` fn must appear in the registrar; there
  is no per-fn opt-out. Registration is harmless (it only makes a route
  available), so the pre-existing gaps are registered, not exempted.
- **The gate** (`server-fn-registrar`, a sibling of the ADR-0053
  `test-backend-pattern` guard) enumerates `web` `#[server]` fns with `syn`
  (`parse_file` + `Visit`, as `xtask/src/coverage/exempt.rs` already does), maps
  each to `PascalCase(fn ident)`, parses the registrar's
  `register_explicit::<web::<vertical>::<Leaf>>()` entries, and fails on any
  missing one. It **matches on `(vertical, leaf)`** — the vertical being the
  first path segment under `web/src` — and rejects a registrar entry of any
  other shape. It checks only the missing direction — a stale registrar entry
  already fails to compile. The core is a pure function unit-tested with string
  fixtures.

  _Amended by #684._ This originally matched by **leaf name alone**, on the
  grounds that re-exports (`pub use listing::*`) make the registrar path differ
  from the source path. Keying on the vertical instead answers that without
  resolving anything: `posts/api.rs` and `posts/api/listing.rs` share a
  vertical, so the glob needs no resolution. It is also the only key the
  registrar can actually spell — every vertical declares `mod api;`
  **privately**, so `web::posts::api::CreatePost` is not a nameable path.
  Leaf-only matching had made the vertical noun in a fn ident load-bearing:
  strip it and `Create`, `Delete`, `List`, `Get`, `Update` all collide across
  verticals.

Reject **A** (leaves the duplication and its own rot) and **B** (no
`inventory`/`linkme` exists in the repo; the cross-rlib linkage that forced
`register_explicit` makes a `linkme` slice's survival uncertain; it would touch
every call site and the coverage-measured `macros` crate — disproportionate for
a gate-caught guarantee).

_Amended by #714._ **B**'s principal cost is now paid. It wanted "a wrapper
attribute macro in the `macros` crate", which did not exist; #714 builds one —
`#[macros::server]` — for unrelated reasons, and every server fn already carries
it. Reopening **B** is
[#731](https://github.com/jaunder-org/jaunder/issues/731), which records the
constraint that must survive any retirement: this gate is also the uniqueness
guard, so it may be dropped only if the placement rule stays enforced or that
check is re-homed.

## Consequences

- A new `#[server]` fn in `web` that is not registered fails `cargo xtask check`
  host-side, naming the fn and its `file:line` — no more silent 404s.
- The second registrar list and its independent rot risk are gone; there is one
  place to keep in sync, and the gate keeps it honest.
- **Two `#[server]` fns with the same ident in one vertical are a hard
  failure.** They collapse to a single `(vertical, leaf)` key, so one registrar
  entry would satisfy both and leave the other to 404 silently — exactly the
  #358 omission this gate exists to catch. Across _different_ verticals the same
  leaf is not a collision at all.

  The compiler does **not** own this case, which is why the check must stay. An
  item defined in `api.rs` silently _shadows_ a glob-imported name of the same
  ident from `pub use listing::*` (verified with `rustc`: exit 0, no error), so
  the pair compiles cleanly; and each vertical's `mod.rs` re-exports an explicit
  list from `api` only, so a duplicate added in `<vertical>/server.rs` never
  reaches a `pub use` conflict either.

  Such a pair would also collide at the `endpoint` level, and after #714 that
  collision is a **compile error**: the placement rule puts every `#[server]` fn
  in `web/src/<vertical>/api.rs` (ADR-0070), so a duplicate `(vertical, leaf)`
  means two items of one name in one module, and the glob re-export that let one
  silently shadow the other is itself deleted. This gate's duplicate check —
  together with the runtime pairwise-distinctness assertion over every generated
  `ServerFn::PATH` (`server/tests/web/server_fn_wire.rs`, which is independent
  of xtask's enumeration) — is therefore belt-and-braces rather than the
  guarantee.

  _Amended by #714._ This paragraph previously read that the endpoint collision
  was "since #684 … enforced independently by the `server-fn-endpoint` gate" —
  "Two guards, not one; neither is redundant, because a `list_mine`/`listMine`
  pair shares a leaf (`ListMine`) while deriving _distinct_ endpoints, so only
  this gate catches it." That gate is deleted (ADR-0082), so this is now the
  only **gate** on the duplicate case; what replaced it is the compiler rather
  than a second gate.

  _Amended by #684._ This bullet previously called leaf collision an "accepted
  limitation … benign", describing it as something that could let an
  unregistered fn **slip through** — a pass. The code hard-**failed** it. The
  prose was wrong, not the code.

- The gate assumes the `#[server(endpoint = "…")]` form (no positional type
  rename); it treats an unexpected positional-rename form as a hard error so the
  assumption cannot silently break the PascalCase mapping.

  _Amended by #714._ The source form is now `#[macros::server(…)]`, which takes
  tracing arguments (`skip_all`, `skip(…)`) alongside the one argument it
  forwards to `#[server]` (`input = …`). Those are `Meta::Path` and `Meta::List`
  respectively, so the positional-rename hard error would fire on 16 of the 55
  sites. The gate therefore considers **only the arguments routed to
  `#[server]`**. Since `input = …` is the sole routed argument and is a
  `Meta::NameValue`, the narrowing preserves positional-rename detection rather
  than defeating it.

- Relocating the router tests to integration keeps `server/src/lib.rs` free of a
  registrar; future router-level assertions belong in the integration suite.
