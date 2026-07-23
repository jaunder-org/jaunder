# Spec — #591: drop the SSR-era full-reload redirect hook; shared reactive session state

- **Issue:** jaunder-org/jaunder#591
- **Milestone:** #8 — Off concurrent SSR (web re-architecture v1)
- **Unblocks:** #592 (post-lifecycle full reloads; depends on this)
- **Date:** 2026-07-22

## Problem

The app is pure Leptos CSR (#179/#239). `leptos_router` 0.8 already intercepts
same-origin `<a>` clicks, so ordinary link navigation is client-side. But two
pieces of SSR-era detritus remain in the login/logout/registration path:

1. **A `set_redirect_hook` override** in `App` (`web/src/pages/mod.rs:79-83`)
   forces every server-fn `redirect(...)` into a full
   `window.location().replace()` document load. Its comment justifies this as
   "refreshing all server-rendered state" — a rationale that is a confirmed
   SSR-era vestige (project owner, 2026-07-22): there is no server-rendered
   state left to refresh, and the Playwright co-rationale is inverted today
   (`end2end/CLAUDE.md` documents `location.replace()` as the _cause_ of
   `waitForURL` unreliability). The override intercepts **three** server fns:
   `login` (`auth/api.rs:95`), `logout` (`auth/api.rs:110`), and `register`
   (`registration/api.rs:129`) — each `redirect("/")`. So every login, logout,
   and registration tears down and re-boots the whole wasm app.

2. **No shared reactive session state to invalidate instead.** Viewer identity
   is fetched ad hoc: five reactive resources call `current_user()` /
   `current_user_is_operator()` (Sidebar reconcile + operator gate
   `web/src/pages/ui.rs:95,80`, Cockpit `web/src/pages/cockpit.rs:35`,
   CreatePostPage `web/src/posts/component.rs:1089`, SubscribeButton
   `web/src/posts/component.rs:1301`), plus direct marker reads
   (`web/src/posts/component.rs:182` viewer==author, `web/src/pages/ui.rs:137`
   boot seed). The full reload survives because nothing else keeps the chrome in
   sync.

## Goals

- Login, logout, and registration complete **without a document reload** — the
  wasm app is not re-booted; chrome (sidebar, topbar) updates reactively.
- One shared, reactive session context replaces the five ad-hoc fetches and the
  direct marker reads.
- Flash-free session identity **and** operator status on boot and first login.
- No comment in the codebase still claims reloads refresh "server-rendered
  state".

## Non-goals (out of scope — deliberately deferred)

- **The post-publish/unpublish `location.replace()` calls** and the permalink
  route misroute → **#592** (blocked on this issue).
- **The `location.replace`/`no-full-load` enforcement gate** → **#592** (ships
  with the last reload-site removal there, not here).
- **The projector** (server-painted public pages, #178) and the **pre-paint
  `/`→`/app` redirect script** — cold-load concerns, not SPA violations. Kept.
- **`require_operator()`** (the server-side operator guard for the backup
  settings server fns) — unchanged; it is the real privilege enforcement.

## Design

### 1. `SessionUser` value + marker schema

Introduce a small, `cfg`-free value type carrying the whole session identity:

```rust
struct SessionUser { username: Username, is_operator: bool }
```

- Lives in a `cfg`-free module reachable by the marker codec, the `session()`
  server fn, and the client context (e.g. `web::auth::session` or alongside
  `web/src/auth/marker.rs`).
- The `jaunder_auth` localStorage marker JSON grows from `{"username": "…"}` to
  `{"username": "…", "is_operator": <bool>}`. `encode_marker` / `decode_marker`
  (`web/src/auth/marker.rs`) round-trip a `SessionUser`.
- **Pre-paint contract preserved:** `PREPAINT_SCRIPT`
  (`web/src/render/mod.rs:38`) reads only `JSON.parse(m).username`; the extra
  top-level `is_operator` field is ignored by it. Therefore `PREPAINT_SCRIPT`,
  its byte-identical `csr/index.html` twin, and the drift test are
  **unchanged**. `username` must remain the top-level key.
- `marker_storage::{get,set,remove}` (`web/src/auth/marker_storage.rs`) operate
  on `SessionUser` (`get() -> Option<SessionUser>`; `set(&SessionUser)`).
- **Backward-compatible decode (required).** Existing sessions already hold
  `{"username":"…"}` with no `is_operator`. `decode_marker` **must** treat an
  absent `is_operator` as `false` (`#[serde(default)]` if the codec moves to
  serde, or the manual equivalent), returning
  `Some(SessionUser{…, is_operator:false})` — **not** `None`. If decode of an
  old marker returned `None`, the boot seed would be `None` and existing
  operators would see the anonymous sidebar flash on the first post-deploy boot
  until `session()` reconciled, violating Goal "flash-free" for in-flight
  sessions. Add a decode test for the field-absent case.
- **Tests that change:** the existing exact-JSON round-trip test
  (`web/src/auth/marker.rs:51-58`, asserting `{"username":"alice"}`) is updated
  to the new `{username, is_operator}` shape; the field-absent decode test is
  added alongside it.

**Security note:** the marker's `is_operator` only decides whether client chrome
_shows_ the operator surface. The operator-gated server fns
(`get_backup_settings`, `update_backup_settings`) independently call
`require_operator()` against the DB, so a hand-edited localStorage flag grants
no privilege — it is cosmetic only.

### 2. `session()` server fn — the one reconcile fetch

```rust
#[server(endpoint = "/session")]
async fn session() -> WebResult<Option<SessionUser>>
```

- Anonymous / expired cookie → `Ok(None)` (reuse the `require_auth` →
  anon-maps-to-None classification currently in `classify_current_user`).
- Authenticated → `Ok(Some(SessionUser { username, is_operator }))`, reading
  `is_operator` from `UserStorage::get_user(auth.user_id)` (as
  `current_user_is_operator` does today).
- **Retires** `current_user()` (`web/src/auth/api.rs:28`) and the reactive
  `current_user_is_operator()` (`web/src/backup/api.rs:46`), plus their
  re-exports / generated wire structs. `require_operator()`
  (`web/src/backup/server.rs`) is kept.

### 3. Shared session context

Provide one context object from `App` (next to the existing `theme`
`provide_context` at `web/src/pages/mod.rs:95`):

- A **seed signal** `RwSignal<Option<SessionUser>>` initialised
  **synchronously** from `marker_storage::get()` at `App` construction →
  flash-free first paint for both identity and operator chrome; synchronously
  readable (serves `viewer==author` and any pre-fetch gate).
- A **reconcile `Resource`** that calls `session()` **keyed on
  `location.pathname`** (per-navigation, preserving today's server-side
  session-expiry detection). On resolve it writes the result back into the seed
  signal **and** into the marker (`set`/`remove`), so the marker stays fresh for
  the next boot. This subsumes the Sidebar reconcile Effect
  (`web/src/pages/ui.rs:95-114`).
- A **mutate helper** to set/clear the session on auth actions (see §5).

Consumers `use_context` this object. Components that need server-confirmed auth
(e.g. CreatePostPage's create-form gate) `.await` the reconcile Resource;
components that only need current identity read the seed signal.

### 4. Consumer conversions

| Site                                      | Today                                         | After                                                                                      |
| ----------------------------------------- | --------------------------------------------- | ------------------------------------------------------------------------------------------ |
| `pages/ui.rs:95` Sidebar reconcile        | own `current_user()` resource + marker Effect | reads context; the reconcile+marker write lives in the context                             |
| `pages/ui.rs:80` operator gate            | own `current_user_is_operator()` resource     | reads `is_operator` from context                                                           |
| `pages/ui.rs:137` boot seed               | `marker_storage::get()` direct                | reads context seed signal                                                                  |
| `pages/cockpit.rs:35`                     | `current_user()` then `list_home_feed`        | reads viewer from context, keeps its own feed fetch                                        |
| `posts/component.rs:1089` CreatePostPage  | own `current_user()` resource                 | awaits context reconcile Resource                                                          |
| `posts/component.rs:1301` SubscribeButton | `current_user()` inside its state resource    | reads viewer from context; keeps its own `is_subscribed_to` fetch keyed on action versions |
| `posts/component.rs:182` viewer==author   | `marker_storage::get()` direct                | reads context seed signal                                                                  |

### 5. Auth flows without the reload

Delete the `set_redirect_hook` override and its stale comment
(`web/src/pages/mod.rs:72-83`). With the override gone, the redirect-hook slot
(a first-caller-wins `OnceLock`) is claimed by `<Router>`, which navigates
client-side.

- `login` / `logout` / `register` server fns keep `leptos_axum::redirect("/")` —
  now a client-side pushState navigation.
- `login()` additionally returns `is_operator` (return type grows from the token
  string to a small struct/tuple carrying `{ token, is_operator }`) so the
  client can write the **complete** marker immediately → flash-free first login.
  The returned token is never read on the client today (only success/failure is:
  `auth/component.rs:26,75`, `registration/component.rs:140`), so this is safe;
  the explicit `Result<String, WebError>` annotation at
  `web/src/auth/component.rs:75` changes with the return type (compiler-forced).
  `register()` stays `-> String` (new users are always `is_operator: false`).
- The client auth-action Effects (`web/src/auth/component.rs:28,100`,
  `web/src/registration/component.rs:56`) update the marker **and** the session
  seed signal via the context mutate helper: login/register → `Some(info)`
  (registration is always `is_operator: false`); logout → `None`.
- The `redirect("/")` navigation changes `location.pathname`, which re-fires the
  per-navigation reconcile Resource — server-confirming the optimistic update
  with no extra wiring.

### 6. Router-hook behavior (settled by leptos_router 0.8.13 source)

Removing the override yields client-side navigation **deterministically** — this
is a code fact, not an assumption:

- `<Router>` installs the redirect hook at mount (`leptos_router` 0.8.13,
  `components.rs:105` `set_redirect_hook`), whose closure calls
  `BrowserUrl::redirect`; for a **same-origin** target (`redirect("/")` is
  same-origin) that resolves to `use_navigate()` → `navigate(...)` inside a
  `request_animation_frame` — the exact client-side pushState we want, with a
  one-tick delay so the Action value updates first.
- The hook slot is a first-caller-wins `OnceLock` (`server_fn` 0.8.12,
  `redirect.rs`). leptos core's `ActionForm`/`Form` register a **full-load**
  fallback hook (`leptos` 0.8.19, `form.rs` → `location().set_href`) _guarded by
  "not already set by a router"_. Every `ActionForm` in this app lives
  **inside** `<Router>`, so Router claims the slot first and the full-load
  fallback never installs. **This ordering (Router-before-form) is the reason
  removal is safe** — state it in the implementing comment where the override
  used to be.

Criterion 1 (no wasm re-boot) still verifies this behaviorally in e2e; no
`use_navigate` fallback code is needed, because Router already _is_ that
fallback.

## Acceptance criteria (observable)

1. **No document reload on auth transitions.** Driving login, logout, and
   registration performs SPA navigation to `/` with **no wasm re-boot** (e.g. a
   value stashed on `window`/module state before the action survives across the
   transition; no full document load fires). Verified by e2e.
2. **Reactive chrome update.** After login the sidebar/topbar show the
   authenticated chrome, and after logout the anonymous chrome, **without** a
   full page load.
3. **Flash-free identity and operator chrome (observable proxy).** A sub-frame
   "no flip" is not deterministically observable in Playwright; verify the
   proxy: on a reload while logged in as an operator, operator chrome is present
   at the **first content-ready paint, before any `session()` response** (assert
   via a CSS locator — not `getByRole`, which skips `display:none` and passes
   vacuously; optionally stall/inspect the `session()` XHR to prove the chrome
   preceded it). This must also hold for an **existing** session whose marker
   predates `is_operator` (see §1 backward-compatible decode).
4. **Single session fetch.** Exactly one server fn (`session()`) backs session
   identity; `current_user()` and the reactive `current_user_is_operator()` no
   longer exist in the tree (`rg` finds no definition or reactive caller).
5. **No stale rationale.** No comment in `web/src` claims a reload refreshes
   "server-rendered state"; the `set_redirect_hook` override is gone.
6. **Marker contract intact.** `PREPAINT_SCRIPT` and its `csr/index.html` twin
   are byte-identical (drift test green); the marker round-trips
   `{username, is_operator}` with `username` at top level.
7. **Gate green.** `cargo xtask check` passes host-side; all four e2e combos
   (`{sqlite,postgres}×{chromium,firefox}`) green in CI.

## E2E + docs

- Rewrite the post-login / post-logout specs: `waitForURL` is now reliable for
  these flows (ordinary pushState). **Caveat for spec authors:**
  `body[data-hydrated]` (`csr/src/lib.rs`) is set once per _document_ — after an
  SPA navigation, waiting on it is trivially-true; **assert on content
  readiness** (the rendered chrome) instead.
- Update the login/logout guidance in `end2end/CLAUDE.md` for the new
  SPA-navigation behavior. Correct the specific line (`end2end/CLAUDE.md:37-39`)
  that today reads _"Server-side 302 redirects (e.g. logout) are reliable with
  `waitForURL`; client-side `location.replace()` redirects (e.g. post-login) are
  not"_ — after this change **all three** (login/logout/register) are
  client-side pushState and `waitForURL` is reliable for each. The
  post-lifecycle (publish/unpublish) `location.replace` guidance is removed by
  **#592**, not here.
  - **Caveat — file is untracked.** `end2end/CLAUDE.md` is **not** version
    controlled; it exists only in the main checkout's working tree (untracked,
    like the other local agent-guidance files) and is therefore absent from this
    branch/worktree. The guidance edit must be made in the main checkout (or the
    file first brought under version control) — it cannot land as part of this
    branch's diff while it stays untracked. Flag to the maintainer at plan time;
    do not silently skip the doc update.
- Add/extend an e2e that asserts the no-reload property (criterion 1) and the
  operator flash-free property (criterion 3, owner-view CSS locator — not
  `getByRole`, which skips `display:none` and passes vacuously).

## Decisions to record

Update **ADR-0044** (auth marker + pre-paint) to reflect: (a) the marker schema
now carries `is_operator`; (b) the pre-paint contract (`username` top-level key)
is unchanged; (c) the app-level shared session context (seed signal + per-nav
reconcile) is the canonical session-state source, superseding ad-hoc
`current_user()` fetches. This is an ADR update, not a new ADR (the CSR/marker
architecture already exists).

## References

- #173 (SSR removal verdict), #179/#239 (CSR client/shell), #181/ADR-0044 (auth
  marker + pre-paint), #198 (agent confusion that surfaced this), #592 (blocked
  follow-on)
- `end2end/CLAUDE.md` (the `location.replace` workarounds — evidence the
  Playwright rationale is inverted)
