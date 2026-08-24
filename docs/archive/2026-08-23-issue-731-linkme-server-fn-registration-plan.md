# Issue #731 Linkme Server-Fn Registration Implementation Outline

> Execute with jaunder-iterate, delegating individual tasks with
> jaunder-dispatch where useful. This outline exists because the work changes
> macro-generated registration, deletes an xtask gate, and depends on a
> host-only cfg/dependency boundary.

## Scope

In:

- Add linkme auto-registration for `#[macros::server]` fns in host/server
  builds.
- Replace the hand registrar list with iteration over the generated slice.
- Delete the `server-fn-registrar` xtask gate and update architecture/ADR prose.
- Preserve placement-rule uniqueness and runtime wire-path distinctness checks.

Out:

- No server-fn URL, span-name, request, or response behavior changes.
- No relaxation of `web/src/<vertical>/api.rs` placement.
- No changes to `server-fn-tracing`, `server-fn-coverage`, or Playwright
  URL-drift gates except direct compile fallout from deleting registrar symbols.
- No compatibility registrar shim or second hand-maintained inventory.

## Task outline

- [x] Task 1: Add the host-only linkme registration surface
  - Contract: `web` owns a `#[linkme::distributed_slice]` of `fn()` registration
    thunks, exported only when `feature = "server"`. `web/Cargo.toml` adds
    `linkme` without exposing a wasm registration path.
  - Verification: a focused host compile/test lane covering `web`'s server
    feature compiles, and a wasm/client lane still compiles without host-only
    registration symbols.

- [x] Task 2: Teach `#[macros::server]` to submit each generated type
  - Contract: for each annotated fn, macro expansion emits one host-only thunk
    submitted to the web-owned slice through a crate-local `crate::…` slice
    path, and that thunk calls
    `server_fn::axum::register_explicit::<GeneratedType>()`. The thunk name must
    be deterministic and collision-free within the generated module.
  - Verification: macro unit tests assert the expansion contains the linkme
    submission, the generated type in the turbofish, and cfg-gating; existing
    macro placement tests remain unchanged.

- [x] Task 3: Replace the hand registrar implementation
  - Contract: `server/tests/helpers::ensure_server_fns_registered()` remains the
    harness entry point and remains idempotent, but iterates the web-owned
    slice. `REGISTERED_SERVER_FN_COUNT` is removed.
  - Verification: focused integration test routes at least one server fn through
    `ensure_server_fns_registered()` with no explicit registration in the test;
    existing router/server-fn integration tests compile against the unchanged
    helper name.

- [x] Task 4: Re-home count/distinctness checks away from the registrar count
  - Contract: `server/tests/web/server_fn_wire.rs` keeps asserting each real
    `ServerFn::PATH` and pairwise distinctness. Its completeness check compares
    to a non-hand-maintained source, preferably the auto-registration slice
    count available in host tests, not `REGISTERED_SERVER_FN_COUNT`.
  - Verification: the wire-path test suite passes and still contains a direct
    pairwise-distinctness assertion over all generated paths.

- [x] Task 5: Delete the registrar gate and its wiring
  - Contract: remove `xtask/src/steps/server_fn_registrar_check.rs`, the
    `lib.rs` module/step registration, and tests/fixtures whose only purpose was
    policing the hand list. Keep shared `web_server_fns` enumeration for
    remaining gates.
  - Verification: `devtool run -- cargo xtask check --no-test` no longer lists
    or runs `server-fn-registrar`; remaining server-fn gates still run.

- [x] Task 6: Update ADR and architecture projection
  - Contract: ADR-0066 records that #731 supersedes the hand registrar/gate with
    linkme auto-registration, while preserving the placement rule and runtime
    wire distinctness backstop. `docs/ARCHITECTURE.md` no longer says there are
    two gates or a hand-maintained registrar.
  - Verification: documentation diff cites the current implementation paths and
    does not contradict ADR-0070, ADR-0081, ADR-0082, or ADR-0120.

- [ ] Task 7: Certify the complete cutover
  - Contract: no `register_explicit::<web::…>()` hand list remains under
    `server/tests`, and adding a new `#[macros::server]` fn would not require an
    xtask registrar fixture edit.
  - Verification: `devtool run -- cargo xtask precommit` passes.

## Risk checks

- Host-only boundary: `server_fn::axum::register_explicit` must not become a
  live wasm/client dependency path; check both host/server and wasm/client
  builds via the smallest existing lanes that exercise those targets.
- Linker survival: the first registration proof must fail if linkme submissions
  are not retained when `web` is linked as an rlib into integration tests.
- Uniqueness: the `#[macros::server]` placement rule remains the primary key; do
  not move or weaken it while deleting the registrar duplicate check.
- Fail-open removal: deleting `server-fn-registrar` must not delete the
  remaining enumeration-count safety for `server-fn-tracing` and
  `server-fn-coverage` without replacing it deliberately.
- Documentation projection: ADR-0066 and `docs/ARCHITECTURE.md` must describe
  the new steady state before ship.
