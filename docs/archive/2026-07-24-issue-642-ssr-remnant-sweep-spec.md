# Spec — #642: Remove SSR-era remnants left behind by the CSR migration (mechanical sweep)

Issue: jaunder-org/jaunder#642 · Milestone: Code quality ratchet · Label: `web`
Companion (already merged): #643 (structural Resource→Effect→signal
dissolution).

## Problem

The full-source SSR-remnant sweep (2026-07-23) found dead machinery, vestigial
APIs, droppable feature flags, and comments that describe the ripped-out SSR
design as current. This issue collects everything **mechanical**: deletion,
mechanical substitution, or comment rewording. **No behavior change anywhere.**

## Current-code reality (post-#643)

#643 landed after the issue was written and shifted the ground:

- **§3 is already done.** #643 deleted `web/src/pages/` wholesale; the
  `read_signal!` macro has zero definitions and zero call sites. Out of scope —
  nothing to do.
- **The four #643 carve-out sites are already done.** AudiencePicker,
  PostCreateForm, UserTimelinePage, EditPostPage (issue #642's §643-owned
  carve-out — _not_ §4) now use client-only `Effect::new` with CSR-worded
  comments. #643 owned and finished them. §4 in this issue is only the two
  PostCard delete/unpublish effects, which is all the `new_isomorphic` work
  left.
- **Two §7 comments moved:** `web/src/pages/mod.rs` comment →
  `web/src/app/component.rs`; `web/src/render/mod.rs` `render_discovery` doc →
  `web/src/app/render.rs`. And §7's two `posts/component.rs` spans (was
  `:1425-1426`, `:1558-1561`) survive as one merged block (~1173-1176) — #643's
  restructuring absorbed the second span.

Line numbers below are the current (post-#643) locations from the 2026-07-24
audit; implementation re-locates by content, since main moves.

## Scope & acceptance criteria

Each criterion is stated so the ship conformance review can tell delivered from
not. The umbrella criterion across all of them: **`git diff` shows only
deletions, mechanical substitutions, and comment rewordings — no logic change**;
and the gate stays green (see Verification). **One sanctioned exception:** the
AC-6 password_reset item may change one read from untracked to tracked if the
investigation finds the untracked read no longer serves a CSR purpose — see that
bullet for its own observable.

### AC-1 — Dead `LeptosOptions` threading removed

- `server/src/commands.rs`: the
  `LeptosOptions::builder().output_name("jaunder") .env(...).site_addr(bind)`
  block (currently ~540-543) is deleted, along with its now-unused
  `Env`/`LeptosOptions` import (~23). `prepare_server` no longer constructs or
  passes leptos options.
- `server/src/lib.rs`: `create_router` no longer takes a
  `leptos_options: LeptosOptions` parameter (~35) and no longer calls
  `.with_state(leptos_options)` (~134). All callers (prod plumbing + tests)
  updated.
- Test mirrors removed: `server/tests/helpers/mod.rs::test_options()` (~93-94)
  and its callers (~344, 459, 673); the inline `LeptosOptions::builder()` in
  `server/tests/web/router.rs` (~57) and `server/tests/misc/commands.rs` (~146);
  every now-unused `use leptos::prelude::LeptosOptions;` import in those three
  test files.
- `server/src/projector/mod.rs` (~53): the doc comment no longer describes
  composing onto "the live `Router<LeptosOptions>`"; reworded to current reality
  (bare `Router`).
- `server/Cargo.toml`: after removal, the `leptos` dep (~27) and dev-dep (~73)
  are trimmed **iff** the crate still compiles — the `ssr` feature is the
  `#[server]`-body gate and is expected to stay. Observable: whatever is
  removed, `cargo xtask check` is green; if nothing can be removed, the dep
  lines are unchanged and a one-line note is added to the issue explaining why.

### AC-2 — SSR-era `LocalSet` test wrappers removed

- `server/tests/misc/commands.rs`: `after_init_server_responds_to_health_check`
  and `prepare_server_binds_and_builds_serving_router` no longer wrap their body
  in a `LocalSet`; the SSR-as-current comments (~155-157, ~201) are gone.
- `server/tests/web/router.rs`: no test body is wrapped in a `LocalSet`; the
  module header (~10) no longer says the tests assert "routing / SSR".
- `home_response_contains_app_content` (~96, the degenerate
  `html.contains("Jaunder")` duplicate of the shell-embed test) is **dropped**.
- Observable: `rg 'LocalSet' server/tests` returns nothing;
  `rg 'home_response_contains_app_content |routing / SSR' server/tests/web/router.rs`
  returns nothing (test dropped + header reworded); the server integration suite
  is green.

### AC-3 — `Effect::new_isomorphic` gone from wasm-only PostCard code

- `web/src/posts/component.rs`: the two PostCard effects (~228 delete, ~236
  unpublish) use `Effect::new`, not `Effect::new_isomorphic`.
- Observable: `rg 'new_isomorphic' web/src` returns nothing (the only remaining
  reference — a comment at ~1174 — is reworded/removed under AC-6). Behavior of
  the delete/unpublish flows is unchanged (a wasm-only file never had a server
  render to schedule against).

### AC-4 — Feature-flag trim (compile-checked)

- `web/Cargo.toml` `server` feature: `leptos_meta/ssr` (~67) and
  `leptos_router/ssr` (~68) are dropped **iff** `cargo xtask check` and a
  `cargo clippy -p web --target wasm32-unknown-unknown -- -D warnings` pass stay
  green. `leptos/ssr` (~66) and `dep:leptos_axum` (~69) are **kept**.
- Observable: either the two sub-crate `ssr` features are removed and the gate
  is green, or they remain with a one-line note on the issue naming what needs
  them.

### AC-5 — `recursion_limit` re-checked

- `server/src/lib.rs` (~1-4): the `#![recursion_limit = "512"]` attribute is
  removed **iff** the full build — including the coverage-instrumented Nix build
  run by `cargo xtask check` — stays green. If the attribute must stay, its
  comment is rewritten to state the _current_ reason (not "monomorphizing
  `web::App`'s route tuple", which this crate no longer does).
- Observable: attribute either gone with a green `cargo xtask check`, or present
  with a comment that no longer cites server-side app rendering.

### AC-6 — Stale-comment sweep (reword only, except the one carved-out item)

Each of these comments no longer asserts retired SSR/hydration mechanics as
live. Rewordings preserve any still-real constraint (e.g. wasm-cleanliness),
changing only the vehicle/rationale. **All items are reword-only except the
password_reset bullet**, which is licensed to change the read itself (see the
umbrella exception above):

- `common/src/mailer.rs:5` and `server/src/mailer/mod.rs:5-6` — "compiled to
  WebAssembly via `web`/`hydrate`": no `hydrate` build exists (wasm entry crate
  is `csr`). Keep the wasm-cleanliness constraint, fix the vehicle.
- `host/src/metrics.rs:1` — "shared by `web` (SSR), …".
- `web/src/posts/api.rs:35` — "SSR-only imports for #[server] bodies".
- `web/src/auth/api.rs:16-17` — "the crate-level SSR dependencies".
- `web/src/app/component.rs:48-51` (moved from `pages/mod.rs`) — "leaks … into
  the hydrated DOM"; CSR mounts a DOM, no hydration.
- `web/src/route_segments.rs:88` — "so SSR route-list / link generation is
  unaffected"; client-side link generation is what matters.
- `web/src/app/render.rs:96-104` (moved from `render/mod.rs`) —
  `render_discovery` doc anchoring behavior to "the reactive SSR render did".
- `web/src/password_reset/component.rs:71-75` — hydration-race rationale for the
  untracked one-shot read (currently `.with_untracked(...)`). **Re-verify**
  whether the untracked read is still wanted under CSR; update the justification
  either way (keep-and-reword, or switch to a tracked read like
  `registration/component.rs:44`). Observable: whichever lands, the comment
  states a real CSR reason, and the token-consuming reset flow still works — a
  password-reset `e2e-local` spot-check is green if the read semantics change.
- `web/src/timeline/component.rs:52-54` — "post-hydration `Effect`".
- `web/src/avatar/component.rs:16-17` — "so SSR and reactive output coincide"
  (the twin is the server _projector_).
- `web/src/cockpit/component.rs:47` — "Client-only effect …" (every effect is
  client-only now).
- `web/src/posts/component.rs:1173-1176` — "never fires server-side" / "would
  needlessly schedule on the server" (one merged block in EditPostPage).
- `server/src/projector/mod.rs:403-408` — "or hydration 404s on projector
  routes"; it's a CSR boot/mount.
- `server/tests/atompub/atompub_rsd.rs:94` — "Rendering the user page
  (server-side) hoists the EditURI …"; discovery links come from the
  shell/projector path now (#198).

Observable: each listed span no longer contains the SSR/hydration-as-current
phrasing; `rg -i 'hydrat|SSR' common/src server/src host/src web/src` returns
only the known-correct residue (comments that describe SSR as _gone_ —
`app/component.rs:76`, `auth/component.rs:111`, `invites/component.rs:18` — the
`csr` crate's `data-hydrated` marker infra, and the `storage/mod.rs` "tags are
hydrated" test-assertion false positive).

## Non-goals (do NOT touch)

- **§643's territory:** the `posts/component.rs` comments/structure at the four
  carve-out sites (already done). The Resource→Effect→signal shapes that remain
  are legitimate CSR data-fetch, not remnants.
- **Explicitly-not-remnants (verified live):** `leptos_axum::redirect("/")`
  (auth/registration api — #591 pushState hook); `set_not_found_status` /
  `ResponseOptions` in `posts/server.rs` (load-bearing 404 semantics asserted by
  integration tests); `leptos_axum` as a dep; `provide_meta_context` /
  `leptos_meta`; `leptos/ssr` and `dep:leptos_axum` features.
- Comments that already describe SSR as _gone_ (`app/component.rs:76`,
  `auth/component.rs:111`, `invites/component.rs:18`) — accurate, leave them.

## Verification

- While iterating: `cargo xtask check` (auto-fixes formatting). The final
  conformance gate is `cargo xtask validate --no-e2e` (verify-only, never
  mutates) — green (host static + clippy + Nix coverage/instrumented tests incl.
  the coverage-instrumented build that AC-1/AC-5 gate on).
- `cargo clippy -p web --target wasm32-unknown-unknown -- -D warnings` green
  (wasm-only lints that host `check` skips, gating AC-3/AC-4).
- Coverage policy respected: no new `cov:ignore` (this is deletion + reword).
- Spot-check: `cargo xtask e2e-local posts` for the PostCard delete/unpublish
  flows touched by AC-3, given the "no behavior change" claim on
  wasm-scheduling.
- The projector↔CSR layout-shift harness (`end2end/tests/layout-shift.ts`) is
  available but **not** required: no task in scope changes what the projector or
  CSR paints, so first-paint congruence is unaffected. Revisit only if a task
  turns out to be paint-adjacent.

## Decisions recorded

- `home_response_contains_app_content` (AC-2): **drop**, not re-point — it
  duplicates the adjacent shell-embed test.
- password_reset untracked read (AC-6): investigation resolved at implementation
  time; comment states a real CSR reason whichever way it lands.
- One PR for the whole sweep (§1,2,4,5,6,7); §3 dropped as already-done.
