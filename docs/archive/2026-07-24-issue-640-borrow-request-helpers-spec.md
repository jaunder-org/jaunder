# Spec — #640: borrow `state` in the test request helpers

**Issue:** jaunder-org/jaunder#640 **Kind:** behaviour-preserving test refactor
(DX / test quality). No production code change; no test _behaviour_ change.

## Problem

The shared HTTP request helpers in `server/tests/helpers/mod.rs` take
`state: Arc<storage::AppState>` **by value**. Because most tests still need
`state` after the call, every call site clones:
`post_form(Arc::clone(&state), …)`. That `Arc::clone(&state)` is pure ceremony
forced by the by-value signature — **269** occurrences across 14 test files. The
mailer variant additionally forces a `mailer.clone() as Arc<dyn MailSender>`
cast at each of its **26** call sites.

The by-value surface is three layers deep (all measured in this worktree):

1. **Shared helpers in `mod.rs`** — 9 functions take `state` by value: the six
   the issue names (`post_form`, `post_json`, `post_form_with_mailer`,
   `post_form_with_secure_flag`, `post_form_with_ua`, `post_form_with_bearer`),
   plus `post_multipart`, the private `post_inner`, **and `make_app`** (14
   direct `make_app(Arc::clone(&state), …)` sites in the atompub/media tests;
   `post_multipart` also calls it internally).
2. **Local per-file wrappers** — `web_posts.rs` has **14** thin wrappers
   (`create_post_json`, `get_post_form`, `delete_post_form`,
   `publish_post_form`, `list_*_form`, …) that take `state` by value only to
   forward it into `post_form` / `post_json`; their callers supply
   `Arc::clone(&state)` (the bulk of web_posts.rs's 106 clones). `web_media.rs`
   has **1** (`media_serve_get`).
3. **Test bodies** call the above directly with `Arc::clone(&state)`.

## Goal

Borrow `state` everywhere it is currently taken by value in the test suite, so
call sites read `post_form(&state, …)` / `create_post_json(&state, …)` with no
clone, and the mailer variant reads `post_form_with_mailer(&state, &mailer, …)`
with no cast. The one unavoidable `Arc::clone` (the router genuinely consumes an
owned `Arc`) moves _inside_ each helper — paid once per helper body, not once
per call site. This is the comprehensive sweep: leave no by-value-`state` helper
behind, so no inconsistent half-state remains in the most-affected file
(`web_posts.rs`).

## Design decisions (resolved)

### D1 — `state`: `&Arc<AppState>`, not `&AppState`

`jaunder::create_router(_, state: Arc<AppState>, _, _, _)` **consumes** an owned
`Arc<AppState>`: it moves `state` into the server-fn closure
(`server/src/lib.rs:62,73`) and clones sub-Arcs (`state.posts.clone()`, …) off
it. The helpers therefore need a real `Arc` to hand down, so the borrow type is
`&Arc<storage::AppState>` (clone once internally), not `&AppState` (which cannot
be re-wrapped into the caller's shared `Arc`). Call sites keep using `state`
after the POST (`state.posts.get_post_by_id(…)`), so the caller's ownership is
retained — exactly what a borrow gives. Local wrappers that merely forward
`state` likewise become `&Arc` and pass it straight through.

### D2 — mailer: generic `&Arc<impl MailSender + 'static>`

All 26 mailer call sites hold a concrete `Arc<CapturingMailSender>` (kept for
post-call assertions like `mailer.sent()`) and pass
`mailer.clone() as Arc<dyn MailSender>`. A non-generic `&Arc<dyn MailSender>`
param would **not** remove the cast: an `Arc<CapturingMailSender>` does not
coerce to `&Arc<dyn MailSender>` through a shared reference. The public helper
is therefore generic —
`post_form_with_mailer<M: MailSender + 'static>(state: &Arc<AppState>, mailer: &Arc<M>, …)`
— and does the single clone-and-unsize (`Arc<M>` → `Arc<dyn MailSender>`)
internally. Call sites become `&mailer`, dropping both the `.clone()` and the
`as Arc<…>` cast. The private `post_inner` keeps taking an owned
`Arc<dyn MailSender>` (the public wrappers already own/produce it —
`noop_mailer()` returns `Arc<dyn MailSender>`).

### D3 — scope: every by-value-`state` helper (24 signatures)

Convert to `&Arc<storage::AppState>`:

- **`mod.rs` (9):** `post_form`, `post_json`, `post_form_with_mailer` (+ generic
  mailer per D2), `post_form_with_secure_flag`, `post_form_with_ua`,
  `post_form_with_bearer`, `post_multipart`, `post_inner` (private; clones once
  for the router), `make_app`.
- **`web_posts.rs` (14):** all local `*_form` / `*_json` /
  `unauthenticated_request` wrappers now taking `state` by value.
- **`web_media.rs` (1):** `media_serve_get`.

`get_asset` (`mod.rs`) is unaffected — it destructures its own local `state`
from a freshly-provisioned `TestEnv` and never receives one as a parameter.

### D4 — the two `async move` closures keep their clone (legitimate residual)

`web_posts.rs:2197` and `web_posts.rs:2272` contain
`let state = Arc::clone(&state);` inside a `move` closure that returns an
`async move` future. The future outlives the closure invocation, so it must
**own** an `Arc` to hold across its `.await` — this clone is required for future
ownership, **not** to satisfy a by-value helper argument. It stays. (Inside the
future, the call becomes `post_json(&state, …)`, borrowing the closure-local
owned clone.) This matches the issue's exact wording: no clone "solely to
satisfy a `post_*` call."

### D5 — the "last-use / move" case is a non-issue

Changing a parameter from by-value to by-reference is strictly more permissive
at the call site: a caller that previously _moved_ `state` into a final call
(genuine last use) simply passes `&state` and lets `state` drop normally
afterward. No call site relies on the _move semantics_ of an `Arc`, so no
behaviour changes. Verified only insofar as the suite still compiles and passes.

## Acceptance criteria

- **AC1** — After the change, no function in `server/tests/` takes
  `state: Arc<storage::AppState>` by value:
  `rg 'state: Arc<storage::AppState>' server/tests/` returns **zero** (the 24
  signatures in D3 now read `state: &Arc<storage::AppState>`). `get_asset` is
  unaffected (it has no `state` parameter).
- **AC2** — `post_form_with_mailer` takes the mailer by generic reference
  (`&Arc<M>` where `M: MailSender + 'static`, i.e.
  `&Arc<impl MailSender + 'static>`).
- **AC3** — No `Arc::clone(&state)` remains in **argument position** (as a
  helper call argument). The only permitted residual is the owned-handle rebind
  `let state = Arc::clone(&state);` for the two `async move` futures in
  `web_posts.rs` (D4). Concretely: after the change,
  `rg 'Arc::clone\(&state\)' server/tests/` returns **exactly 2** lines, both
  `let state = Arc::clone(&state);` (down from 269).
- **AC4** — The `mailer.clone() as Arc<dyn …MailSender>` cast is gone from every
  call site: `rg 'as Arc<dyn.*MailSender>' server/tests/` returns **nothing**
  (down from 26).
- **AC5** — Behaviour preserved: no assertion, request shape, URI, body, or test
  outcome changes; the only edits are helper signatures and the mechanical
  call-site updates (`Arc::clone(&state)` → `&state`; `mailer.clone() as Arc<…>`
  → `&mailer`).
- **AC6** — `cargo xtask validate` is green.

## Out of scope

- Any change to `jaunder::create_router` or other production code.
- The `#635` seeding-layer work (separate issue/layer).
- Borrowing anything other than `state` and the mailer (`storage: &TempDir` is
  already a borrow; `body`/`uri`/`cookie` are unchanged).
