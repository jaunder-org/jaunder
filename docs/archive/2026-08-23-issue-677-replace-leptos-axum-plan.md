# Issue #677 — replace leptos_axum implementation outline

> Execute with jaunder-iterate. Use jaunder-dispatch for independently
> verifiable slices if useful. This outline exists because the approved spec
> changes the server-function integration boundary and touches auth/session
> redirect semantics.

## Scope

In:

- Replace `leptos_axum`'s current server-function bridge with a Jaunder-owned
  adapter over `server_fn::axum`.
- Migrate all current `leptos_axum` call sites to the adapter or to direct
  `server_fn` constants where appropriate.
- Remove `leptos_axum` from manifests and lockfile, while retaining `server_fn`
  axum registration/dispatch.
- Update architecture/comments that name `leptos_axum` as current behavior.
- Verify dependency graph, static checks, and focused e2e paths named by the
  spec.

Out:

- No server-function URL, endpoint derivation, or `#[macros::server]` placement
  change.
- No REST/API replacement for Leptos server functions.
- No auth/session behavior change; session establishment remains cookie-only.
- No broad web component, projector, or Playwright refactor.

## Task outline

- [x] Task 1: Add the local server-function adapter
  - Contract: introduce a small adapter boundary owned by Jaunder, with Axum
    dispatch in `server` and server-function body helpers in `web`:
    - `handle_server_fns_with_context(additional_context, req) -> Response<Body>`
      or an equivalently named handler called from `server/src/lib.rs`.
    - `ResponseOptions` context type supporting at least `set_status`,
      `insert_header`, and header/status merge into the final response.
    - `extract<T>()` over `axum::extract::FromRequestParts<()>`, using cloned
      `axum::http::request::Parts` from Leptos context.
    - `redirect(path)` preserving current `Location` +
      `server_fn::redirect::REDIRECT_HEADER` behavior for enhanced requests and
      302 for HTML/form requests.
  - Contract: handler preserves the plain-form fallback currently supplied after
    `service.run(req)`: when `Accept` contains `text/html`, a `Referer` exists,
    and no `Location` was set explicitly, return `302 Found` to the referer.
  - Contract: handler dispatches via
    `server_fn::axum::get_server_fn_service(path, method)`; missing service
    returns the same class of 400 response as upstream.
  - Contract: handler creates a Leptos `Owner` and holds it across the
    server-function future with `ScopedFuture`; it provides request `Parts`,
    `ResponseOptions`, and the caller's DI closure before `service.run(req)`.
  - Verification: focused Rust tests where practical for `ResponseOptions` merge
    and redirect header/status behavior; otherwise covered by Task 4 focused
    e2e.

- [x] Task 2: Migrate call sites and manifests
  - Contract: `server/src/lib.rs` routes `/api/{*fn_name}` through the new
    adapter and still provides `AppState`, mailer, and `CookieSettings`
    contexts.
  - Contract: `web/src/auth/api.rs`, `web/src/registration/api.rs`,
    `web/src/media/api.rs`, `web/src/auth/server.rs`, and
    `web/src/posts/server.rs` import the local adapter surface instead of
    `leptos_axum`.
  - Contract: root `Cargo.toml`, `server/Cargo.toml`, and `web/Cargo.toml`
    remove `leptos_axum`; production dependencies enable `server_fn` with the
    axum backend where the adapter compiles. `web` keeps `server_fn` multipart
    support required by `MultipartData`.
  - Verification: `leptos_axum` search over `web/src`, `server/src`,
    `Cargo.toml`, `server/Cargo.toml`, `web/Cargo.toml`, and `Cargo.lock` is
    empty after Cargo updates.

- [x] Task 3: Update architecture and comments
  - Contract: `docs/ARCHITECTURE.md` no longer says the Leptos SSR stack is
    still present through `leptos_axum`; it names the Jaunder adapter as the
    current `/api` server-function invocation path and records that the adapter
    owns the root-owner/context lifetime responsibility.
  - Contract: source comments in `web/src/error/server.rs`,
    `web/src/auth/server.rs`, and `web/src/media/api.rs` use the new adapter
    name and keep the same invariant wording where behavior is unchanged.
  - Verification: `leptos_axum` search across docs/source only finds historical
    ADR/archive references if any; current architecture and current source do
    not name it as live behavior.

- [x] Task 4: Verify the replacement boundary
  - Contract: run static and behavioral proof in this order, stopping on the
    first real failure and fixing source rather than suppressing lints:
    1. `devtool run -- cargo xtask check --no-test`
    2. `devtool run -- cargo tree -e features -p web --features server`
    3. focused e2e flows for auth login/logout/registration no-full-reload
       redirects, media upload, and viewer/post optional-auth/not-found status
       coverage.
  - Contract: inspect the parked `cargo tree` output and final notes must state
    whether `tachys/ssr` and hydration are absent from the relevant feature
    tree.
  - Verification: the focused e2e commands may use existing xtask/e2e selection
    if available; if no narrow selector exists, use the smallest existing e2e
    command that exercises the named specs.

## Risk checks

- Owner/context lifetime: no server-function body should need read-before-await
  discipline after the swap; the adapter must hold the root owner for the whole
  service future.
- Redirect semantics: enhanced actions must receive `serverfnredirect` without a
  forced 302; otherwise login/logout/register can full reload or fail body
  deserialization.
- Response mutation: cookie headers and not-found status are applied after
  `service.run(req)`, not before, and header insertion semantics match current
  cookie helpers.
- Request extraction: only request-head/extension extractors are promised; do
  not add body-consuming extraction behavior.
- Dependency graph: `server_fn/ssr` is expected for inventory; `leptos/ssr`,
  `tachys/ssr`, and hydration from `leptos_axum` are the unwanted edges.
- Architecture view: `docs/ARCHITECTURE.md` is part of the delivered state, not
  follow-up cleanup.
