# Issue #731 — retire the hand server-fn registrar with linkme auto-registration

## Outcome

Server integration/router tests no longer depend on a hand-maintained list of
`register_explicit::<web::…>()` calls. Every `#[macros::server]` function
contributes a registration thunk to a `linkme` distributed slice, and the test
harness registers by iterating that slice.

The old `server-fn-registrar` xtask gate is removed only after its remaining
safety role is covered elsewhere: placement stays enforced by
`#[macros::server]`, and runtime wire assertions continue to prove every
generated `ServerFn::PATH` is pairwise distinct.

## Load-bearing decisions

- Use `linkme` in the `web` crate as the registration transport. The macro may
  emit slice entries, but the slice itself belongs to `web` so call sites can
  use `crate::…` paths and test code can iterate one public test-support
  surface.
- Each slice entry is a no-argument registration thunk that calls
  `server_fn::axum::register_explicit::<GeneratedType>()`. This preserves the
  existing registration API and avoids inventing a second registry format.
- Prove the rlib/linker premise before deleting the hand list: at least one
  integration test must route through a server fn after registration comes
  solely from the distributed slice.
- Keep linkme registration host-only. The emitted thunks call
  `server_fn::axum::register_explicit`, so the slice, thunk statics, and harness
  iterator must compile only for the `web/server` host build; the wasm client
  must not gain a live axum-registration path.
- Delete `server/tests/helpers/registrar.rs`'s hand list and
  `REGISTERED_SERVER_FN_COUNT`; do not keep a second count/list as a
  compatibility shim.
- Delete `xtask/src/steps/server_fn_registrar_check.rs` and remove the
  `server-fn-registrar` gate from the xtask step list. The omission it checked
  is made unrepresentable by macro emission.
- Keep the `#[macros::server]` placement rule unchanged: every server fn must
  live in `web/src/<vertical>/api.rs`. That rule is the uniqueness guard that
  makes `(vertical, ident)` a compiler-enforced primary key.
- Keep `server/tests/web/server_fn_wire.rs` as the runtime backstop for the real
  generated URLs, including pairwise distinctness. It should stop depending on
  the removed registrar count and instead compare against the auto-registration
  slice count or another non-hand-maintained count.
- Update ADR-0066 and `docs/ARCHITECTURE.md` so current architecture no longer
  says a hand registrar and registrar gate exist.

## Acceptance

- Adding a new `#[macros::server]` fn no longer requires editing
  `server/tests/helpers/registrar.rs` or any xtask registrar fixture.
- No `register_explicit::<web::…>()` hand list remains under `server/tests`.
- `ensure_server_fns_registered()` still exists as the harness entry point, but
  its host-only implementation iterates the `linkme` slice.
- A focused integration test proves a server fn route is reachable after
  linkme-driven registration, with no explicit type registration in that test.
- A wasm/client build still compiles without exposing the axum-registration
  thunks outside the `server` feature boundary.
- `server_fn_wire::server_fn_wire_paths_are_pairwise_distinct` or an equivalent
  test still fails if two generated server-fn paths collide.
- The removed xtask gate is absent from `cargo xtask check` / `precommit` step
  registration and from architecture documentation.
- `devtool run -- cargo xtask precommit` passes.

## Boundaries

- Do not relax the `web/src/<vertical>/api.rs` placement rule.
- Do not change server-fn wire URLs, span names, or request/response semantics.
- Do not alter `server-fn-tracing`, `server-fn-coverage`, or Playwright
  URL-drift gates except for compile fallout directly caused by deleting the
  registrar gate.
- Do not add a deprecated registrar shim or a second hand-maintained server-fn
  inventory.
