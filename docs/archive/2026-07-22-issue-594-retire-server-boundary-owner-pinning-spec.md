# Spec — Retire `server_boundary`'s SSR-vestigial owner-pinning (#594)

## Summary

Investigation issue #594 asked whether the leptos owner-pinning in
`web/src/error.rs`'s `server_boundary` / `owner_ancestry_strong` — the #89 →
#124 → #138 mechanism — is still load-bearing now that the app is CSR-only (no
component SSR; #487, #515). **Determination: it is vestigial.** This spec
records that finding and the removal it authorizes.

## Determination (the investigation result)

The owner-pinning survives an owner being dropped while a `#[server]`-fn future
is suspended at an `.await`. Every scenario that made that drop possible is
SSR-specific, and all are gone:

| ADR-0016 addendum | Load-bearing scenario                                                                                                   | Status now                                                                         |
| ----------------- | ----------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------- |
| #89               | context lost across an `.await` in a server-fn body                                                                     | The `/api` path is _already_ protected by leptos_axum, which establishes the owner |
| #124              | server fn invoked from an **SSR `Resource` fetcher**, first polled on a worker after the owner's strong ref was dropped | Path removed — `server_resource` deleted in #515                                   |
| #138              | ancestor (SSR-root) owner dropped during **page-render SSR**                                                            | Path removed — no page-render SSR (#487)                                           |

The **sole remaining invocation path** is a browser HTTP `POST /api/{fn}`
dispatched by `leptos_axum::handle_server_fns_with_context`. In leptos_axum
0.8.9 (`handle_server_fns_inner`), that path:

- creates one `Owner::new()` at the axum-handler entry, where there is no
  ambient leptos owner — so the owner is **parentless (a root)**;
- runs `additional_context()` (our `provide_app_state_contexts`) and the
  server-fn body **inside that one owner**, re-applied on every poll by
  leptos_axum's own `ScopedFuture` and held strong by the `owner` stack local
  for the entire `.await`.

Consequently, inside `server_boundary`:

- `owner_ancestry_strong` walks `Owner::parent()` from a root → `None` → returns
  an **empty vec** (holds nothing);
- the inner `ScopedFuture::new_untracked` **duplicates** leptos_axum's outer
  `ScopedFuture` and adds nothing over the stack-held strong ref.

The owner-pinning is therefore dead weight on the only path that exists. The
**error-projection half** of `server_boundary` (`emit_boundary_failure` +
`project` mapping `InternalError → WebError`) is unrelated to SSR and is
retained unchanged.

## In scope

Removal of the vestigial owner-pinning and the documentation it leaves stale.
The change is confined to `web/src/error.rs` and `docs/adr/0016-…md`; the
`boundary!` macro (`web/src/lib.rs`) and every call site are untouched because
`server_boundary`'s signature does not change.

## Out of scope

- The `boundary!` macro, `emit_boundary_failure`, `project`, and every server-fn
  call site — unchanged.
- Milestone endgame work (#520: dropping js-sys/wasm-bindgen, `target_arch`
  cleanup, retiring `client_only`). This removal is a step toward it but does
  not do it.
- Any change to `host`'s error carrier.

## Acceptance criteria

Each is stated so ship-time conformance review can tell delivered from not.

1. **AC1 — `server_boundary` no longer pins owners.** After the change,
   `web/src/error.rs`'s `server_boundary` contains no reference to
   `Owner::current`, `owner_ancestry_strong`, or `ScopedFuture`; its body
   reduces to awaiting the future once and, on `Err`, calling
   `emit_boundary_failure` then returning `project(kind, public_message)`. A
   repo-wide search
   (`rg 'ScopedFuture|owner_ancestry_strong|Owner::current' web/src/error.rs`)
   returns no matches.

2. **AC2 — `owner_ancestry_strong` is deleted.** The free function
   `owner_ancestry_strong` no longer exists in `web/src/error.rs`.

3. **AC3 — the `owner_lifetime` test module is deleted in full.** The entire
   `#[cfg(test)] mod owner_lifetime { … }` block (module doc + all six tests,
   including the four exercising leptos `Owner`/`ScopedFuture` primitives) is
   removed. This deliberately goes beyond the issue's literal "remove the
   `owner_lifetime` `server_boundary_*` tests": the four leaf tests characterize
   leptos `Owner`/ `ScopedFuture` primitives, not our code, so once the
   mechanism they document is retired they test leptos internals and are removed
   with the rest of the module.

4. **AC4 — the error-projection behavior is unchanged and still proven.** Every
   remaining `server_boundary`/`project` test in `web/src/error.rs`'s `tests`
   module (Ok pass-through, `InternalError → WebError` projection for each kind,
   source-chain masking, tracing-field evaluation) is retained and passes. No
   public API or wire form changes.

5. **AC5 — stale doc comments are corrected.** The `server_boundary` doc/`//`
   comment block no longer describes the #89/#138 owner-pinning as current
   behavior; it reflects that the body is awaited directly and only the error
   projection remains.

6. **AC6 — ADR-0016 records the retirement.** A new dated addendum in
   `docs/adr/0016-dependency-injection-and-appstate.md` states the #89/#124/#138
   owner-pinning is retired, gives the reason (no component SSR → only the
   `/api` path → leptos_axum owns the owner-lifetime guarantee → the wrap is
   redundant), and marks the earlier #89/#124/#138 addenda as
   superseded/historical so a future reader does not trust them or reintroduce
   the wrap. Note: the #138 addendum already cites a test
   (`post_await_read_loses_ancestor_context_when_parent_owner_dropped`) that
   does not exist in `error.rs` (pre-existing doc drift) — the new addendum
   supersedes it; do not attempt to preserve that dangling reference.

7. **AC7 — the gate is green.** `cargo xtask validate --no-e2e` passes on the
   branch (static + clippy + coverage), confirming no dead imports, no coverage
   regression from the removed tests, and no clippy fallout.

## Non-goals / explicitly not added

- No replacement regression test is added. The surviving path's owner-lifetime
  guarantee belongs to leptos_axum, not our code; modeling it in a test would
  test leptos_axum internals. `server_boundary`'s remaining behavior stays
  covered by the existing projection/emit tests (AC4).
