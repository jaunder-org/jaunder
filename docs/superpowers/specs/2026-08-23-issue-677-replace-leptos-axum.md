# Issue #677 — replace leptos_axum with server_fn::axum

## Outcome

Jaunder's server build no longer depends on `leptos_axum` or the Leptos SSR
rendering stack it forces in. The `/api/<vertical>/<op>` server-function route
still behaves the same for the CSR client: dependency-injected
storage/mailer/cookie settings are available, request-head extractors work,
response cookies/status mutations survive, and enhanced action redirects stay
client-side.

## Load-bearing decisions

- This is a behavioral-preserving integration replacement, not a server-function
  wire redesign. ADR-0082's `/api/<vertical>/<op>` namespace,
  `#[macros::server]` placement rule, and existing `ServerFn::PATH` tests stay
  unchanged.
- Replace `leptos_axum` with a small Jaunder-owned server-function adapter over
  `server_fn::axum`, because `server_fn::axum::handle_server_fn` can dispatch
  requests but cannot provide Jaunder's DI contexts, request `Parts`, response
  mutation context, or owner-lifetime wrapper.
- The adapter is allowed to clean up the boundary instead of pretending to be
  `leptos_axum`, but it must own the same four load-bearing capabilities Jaunder
  currently uses: contextual handler dispatch, request-head extraction, redirect
  signaling, and response option merging.
- The `/api/{*fn_name}` axum route must still dispatch through
  `server_fn::axum::get_server_fn_service(path, method)`, so server-function
  inventory registration remains `server_fn/ssr` + `inventory`, not a Jaunder
  registry.
- The handler must create and hold a Leptos `Owner` across the whole
  server-function future with the same owner-lifetime property the current
  `leptos_axum` path provides. ADR-0016's current state depends on the `/api`
  path holding contexts across awaits; do not reintroduce read-before-await
  discipline.
- The handler must provide cloned request `Parts` in Leptos context before any
  server-function body runs. Existing auth, optional auth, `HeaderMap`, and
  `Extension<Arc<PathBuf>>` reads are request-head/extension extraction only;
  consuming body extraction remains out of scope.
- Response mutation is a local context object, not
  `server_fn::response::Res::redirect`. It must support the existing
  cookie/status use cases and merge status/headers into the final axum
  `Response<Body>` after the selected server-function service runs.
- Redirect semantics must match the current CSR contract: set `Location` for all
  redirects; for enhanced server-function requests, set
  `server_fn::redirect::REDIRECT_HEADER` without forcing an HTTP 302 so the
  client can deserialize the response and then navigate through the redirect
  hook; for HTML/form requests, return a normal temporary redirect.
- Keep the current plain-form referer fallback if the replacement handler would
  otherwise drop a behavior `leptos_axum` supplied for `Accept: text/html`
  requests with no explicit `Location`.
- Dependency cleanup must remove `leptos_axum` from the workspace/server/web
  manifests and remove `web/server`'s `dep:leptos_axum` feature edge.
  `server_fn::axum-no-default` may be enabled where the adapter needs the axum
  backend; `server_fn/ssr` is expected and is not the dead rendering stack.
- Update comments that name `leptos_axum` as the sole server-function invocation
  path so they describe the new Jaunder adapter and the same owner/context
  invariant.
- Update `docs/ARCHITECTURE.md` where it currently states that `leptos_axum`
  supplies the only server-function invocation path and that the Leptos SSR
  stack is still present through `leptos_axum`.
- No new ADR is required for this issue. The durable decisions are already
  covered by ADR-0040 (CSR, no server-side UI render), ADR-0016 (DI contexts),
  and ADR-0082 (server-function wire namespace); this issue swaps an
  implementation bridge inside those decisions.

## Acceptance

- `leptos_axum` has no references in `web/src`, `server/src`, `web/Cargo.toml`,
  `server/Cargo.toml`, the workspace dependency table, or `Cargo.lock`.
- `cargo tree -e features -p web --features server` shows no `tachys/ssr` and no
  Leptos hydration stack pulled in by `leptos_axum`, while server functions
  still register and dispatch through `server_fn`.
- `cargo xtask check --no-test` passes after the manifest/code cleanup.
- Existing auth e2e flows still pass for login, logout, registration, session
  cookie set/clear, and client-side no-full-reload redirect behavior.
- Existing media upload e2e flows still pass, proving multipart server functions
  still receive request extensions and auth context.
- Existing viewer/post flows that depend on optional auth and not-found status
  still pass, proving request `Parts` and response status mutation still work.
- A focused dependency proof is captured in the final verification notes: the
  command used to inspect the feature graph and the absence of
  `tachys/ssr`/hydration from the relevant tree.
- `docs/ARCHITECTURE.md` no longer describes `leptos_axum` as current
  architecture and instead names the Jaunder-owned `server_fn::axum` adapter and
  its owner/context responsibility.

## Boundaries

- No server-function URL change, endpoint derivation change, TypeScript endpoint
  migration, or Playwright route rewrite.
- No REST API replacement and no migration away from Leptos server functions.
- No return-shape or auth/session semantics change; web session establishment
  remains cookie-only.
- No broad component, projector, or routing cleanup beyond edits needed to
  remove `leptos_axum` cleanly.
- No dependency version upgrade unless required to make the locked
  `server_fn::axum` backend compile; prefer the already locked
  `server_fn 0.8.12` API.
- No full e2e matrix requirement for this issue unless the focused e2e proof
  exposes backend/browser-specific behavior.
