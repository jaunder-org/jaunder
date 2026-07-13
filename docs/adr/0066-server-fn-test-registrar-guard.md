# ADR-0066: Guard the server-fn test registrar with an xtask check

- Status: proposed
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
  `register_explicit::<web::…::LEAF>()` leaf names, and fails on any missing
  leaf. It **matches by leaf type name, not module path**, because re-exports
  (`pub use listing::*`) make the registrar path differ from the source path. It
  checks only the missing direction — a stale registrar entry already fails to
  compile. The core is a pure function unit-tested with string fixtures.

Reject **A** (leaves the duplication and its own rot) and **B** (no
`inventory`/`linkme` exists in the repo; the cross-rlib linkage that forced
`register_explicit` makes a `linkme` slice's survival uncertain; it would touch
every call site and the coverage-measured `macros` crate — disproportionate for
a gate-caught guarantee).

## Consequences

- A new `#[server]` fn in `web` that is not registered fails `cargo xtask check`
  host-side, naming the fn and its `file:line` — no more silent 404s.
- The second registrar list and its independent rot risk are gone; there is one
  place to keep in sync, and the gate keeps it honest.
- **Accepted limitation:** leaf-name matching means two `#[server]` fns with the
  same name in different modules collapse to one leaf, so the gate could miss
  one being unregistered. This is benign — they would also collide at the
  `endpoint` level — and is preferred over resolving re-exports to reconstruct
  full paths.
- The gate assumes the `#[server(endpoint = "…")]` form (no positional type
  rename); it treats an unexpected positional-rename form as a hard error so the
  assumption cannot silently break the PascalCase mapping.
- Relocating the router tests to integration keeps `server/src/lib.rs` free of a
  registrar; future router-level assertions belong in the integration suite.
