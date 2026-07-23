# Plan — #324: converge the `profile` vertical onto the file-level host/wasm split

**Spec:** `docs/superpowers/specs/2026-07-23-issue-324-web-profile-colocate.md`
(read it for what/why; this plan is how). **For agentic workers:**
`jaunder-iterate` drives execution, delegating a task to `jaunder-dispatch` when
useful; tick checkboxes in real time.

## Review header

**Goal.** Move the `profile` web vertical onto the canonical file-level split —
`mod.rs` (wiring) / `api.rs` (endpoints + DTO + test) / `component.rs`
(wasm-only UI) — deleting `pages/profile.rs`, and modernize the one lingering
`<ActionForm>` control to the ADR-0065 typed direct-bind pattern.

**Scope.**

- _In:_ create `profile/api.rs` + `profile/component.rs`; reduce
  `profile/mod.rs` to wiring; delete `pages/profile.rs` + rewire `pages/mod.rs`;
  modernize `DefaultPostFormatControl`; update the two stale "ActionForm"
  comments and the #498 e2e locator.
- _Out:_ #330 (App/Router move), #312 (dissolve `pages/ui.rs`), the `email`
  vertical / `/profile/email` route, any `#[server]` fn semantic change. No new
  ADR.

**Tasks (one line each).**

1. Extract `profile/api.rs` (DTO + 4 `#[server]` fns + server use-block + wire
   test) out of `mod.rs`; `mod.rs` re-exports them. Tree stays green.
2. Move the UI verbatim into wasm-only `profile/component.rs` (incl. the
   existing `<ActionForm>` control); `mod.rs` gains the `//!` doc + gated
   `component` wiring; delete `pages/profile.rs`, drop `pub mod profile;`,
   repoint the router import.
3. Modernize `DefaultPostFormatControl` to typed direct-bind; refresh the two
   stale "ActionForm" comments and the #498 e2e locator; verify the profile e2e
   locally.

**Key risks / decisions.**

- **Green at every commit:** Task 1 keeps `pages/profile.rs` + `email` compiling
  via `mod.rs` re-exports; Task 2 repoints the router in the same commit that
  deletes the old file. Never leave an unresolved `crate::profile::` path.
- **wasm-only surface:** after Task 2 the components no longer host-compile, so
  `wasm-clippy` (`--target wasm32-unknown-unknown -D warnings`) is load-bearing
  — run it before committing Tasks 2 and 3, not just host clippy.
- **Endpoint decode test survives modernization:** the `serde_qs` unit test
  guards the endpoint's wire contract (`format=<token>`), independent of the
  client widget — it moves to `api.rs` unchanged and is _not_ weakened by Task 3
  (only its comment changes). Spec criterion 9.
- **No new coverage debt:** moving the UI to wasm-only removes it from host
  coverage (expected, ADR-0070); **add no `cov:ignore` and lean on no
  `#[component]` exemption** to force host compilation. Spec criterion 4.

## Global constraints

- **No `Co-Authored-By` trailer** on any commit.
- **Before each commit:** run `cargo xtask check` clean (the pre-commit hook
  runs the full check; `jaunder-commit`). For the server-gated `api.rs` code,
  also confirm `cargo check -p web --all-features --all-targets` — the default
  check path can skip `feature = "server"` web code.
- **Before committing Tasks 2 & 3 (wasm-only code):**
  `cargo clippy -p web --target wasm32-unknown-unknown -- -D warnings`.
- **Import discipline:** import enough that call sites read without `crate::…`
  long prefixes; co-located `component.rs` should reach the endpoints via
  `super::api::{…}`.
- Follow `CONTRIBUTING.md` (coverage policy, web conventions) and
  `docs/web-style-guide.md` §8 (the vertical layout template).

---

## Task 1 — Extract `profile/api.rs`; `mod.rs` re-exports

**Files:**

- **Create** `web/src/profile/api.rs`: move, essentially verbatim, from
  `profile/mod.rs`:
  - the shared `use` lines the endpoints need (`WebResult`, the `common::*`
    newtypes, `serde::{Deserialize, Serialize}`),
  - the grouped `#[cfg(feature = "server")] use { … };` block,
  - `pub struct ProfileData { … }`,
  - the four `#[server]` fns (`get_profile`, `update_profile`,
    `get_default_post_format`, `set_default_post_format`) unchanged,
  - the `#[cfg(test)] mod tests { … }` wire-decode test (its
    `use super::SetDefaultPostFormat;` resolves correctly — `super` is now the
    `api` module where the struct lives).
- **Edit** `web/src/profile/mod.rs` → reduce to:

  ```rust
  mod api;

  pub use api::{
      get_default_post_format, get_profile, set_default_post_format, update_profile,
      GetDefaultPostFormat, GetProfile, ProfileData, SetDefaultPostFormat, UpdateProfile,
  };
  ```

  (The `//!` module doc + `component` wiring arrive in Task 2 — mod.rs is
  already "wiring only" after this task, just not yet complete.)

**Why green:** `pages/profile.rs` still does
`use crate::profile::{get_default_post_format, get_profile, SetDefaultPostFormat, UpdateProfile};`
and `email/component.rs` still does `use crate::profile::get_profile;` — all
satisfied by the re-exports. No consumer edits.

**Check:**

- `cargo check -p web --all-features --all-targets` → PASS.
- `cargo nextest run -p web set_default_post_format_wire_rejects_unknown_token`
  → PASS (test now compiled from `api.rs`).
- `cargo xtask check` clean, then commit:
  `refactor(web/profile): extract api.rs (endpoints + DTO + wire test) from mod.rs`.

## Task 2 — Move UI into wasm-only `component.rs`; rewire `pages/`

**Files:**

- **Create** `web/src/profile/component.rs` (`#[cfg(target_arch = "wasm32")]`
  applied at the `mod` line in `mod.rs`, **not** inside this file): move
  `ProfilePage` and the private `DefaultPostFormatControl` verbatim from
  `pages/profile.rs`. Only import changes:
  - `use crate::topbar::Topbar;` (was `use crate::pages::Topbar;`),
  - reach the endpoints/actions via
    `use super::api::{get_default_post_format, get_profile, SetDefaultPostFormat, UpdateProfile};`,
  - keep `crate::error::WebError`, `crate::forms::Field`, the `common::*` and
    `leptos::prelude::*` imports.
  - **Do not** change `DefaultPostFormatControl` yet — the `<ActionForm>`
    control moves unchanged (behavior-preserving move; modernization is Task 3).
- **Edit** `web/src/profile/mod.rs` → final wiring shape: add an ADR-0070-style
  `//!` doc (mirror `auth/mod.rs`: what lives in `api`/`component`, wiring-only
  note), then:

  ```rust
  mod api;
  /// The wasm-only profile UI (`ProfilePage`) — never host-compiled (ADR-0070).
  #[cfg(target_arch = "wasm32")]
  mod component;

  pub use api::{ … as in Task 1 … };
  #[cfg(target_arch = "wasm32")]
  pub use component::ProfilePage;
  ```

- **Delete** `web/src/pages/profile.rs`.
- **Edit** `web/src/pages/mod.rs`: remove `pub mod profile;` (line 1); remove
  `use crate::pages::profile::ProfilePage;` (line 25) and instead import
  `ProfilePage` from the vertical — fold into the existing `crate::…` import
  cluster as `use crate::profile::ProfilePage;`. The
  `<Route path=StaticSegment("profile") view=ProfilePage />` line is unchanged;
  the `/profile/email` → `EmailPage` route is untouched.

**Check (spec criteria 1–7):**

- `rg -n "target_arch" web/src/profile` → matches only the two wiring lines in
  `mod.rs` (criterion 5); `rg -n "cov:ignore|component-exempt" web/src/profile`
  → none (criterion 4).
- `cargo check -p web --all-features --all-targets` → PASS (host: no
  `ProfilePage`; api.rs compiles).
- `cargo clippy -p web --target wasm32-unknown-unknown -- -D warnings` → PASS
  (wasm: `ProfilePage` + control compile).
- `test -e web/src/pages/profile.rs` → absent; `rg "pages::profile" web/src` →
  none.
- `cargo xtask check` clean, then commit:
  `refactor(web/profile): move UI into wasm-only component.rs; drop pages/profile.rs`.

## Task 3 — Modernize `DefaultPostFormatControl` + refresh tests

**Files:**

- **Edit** `web/src/profile/component.rs` → rewrite `DefaultPostFormatControl`
  to the ADR-0065 direct-bind pattern (mirror the audience `<select>` in
  `posts/component.rs` ~355–377):
  ```rust
  #[component]
  fn DefaultPostFormatControl() -> impl IntoView {
      let action = ServerAction::<SetDefaultPostFormat>::new();
      let initial = Resource::new(|| (), |()| get_default_post_format());
      // Signal created OUTSIDE Suspense and seeded inside — the same shape as
      // ProfilePage's dn_field/bio_field (created outside, `.set` inside), so the
      // control's owner is the component, not the transient Suspend scope.
      let format = RwSignal::new(PostFormat::Html);
      let options = [
          (PostFormat::Markdown, "Markdown"),
          (PostFormat::Org, "Org"),
          (PostFormat::Html, "HTML"),
      ];
      view! {
          <Suspense fallback=|| ()>
              {move || Suspend::new(async move {
                  format.set(initial.await.unwrap_or(PostFormat::Html));
                  view! {
                      <label class="j-field-label" for="default-post-format">
                          "Default post format"
                      </label>
                      <select
                          id="default-post-format"
                          class="j-field-val"
                          on:change=move |ev| {
                              if let Ok(f) = event_target_value(&ev).parse::<PostFormat>() {
                                  format.set(f);
                              }
                          }
                      >
                          {options
                              .into_iter()
                              .map(|(f, label)| view! {
                                  <option value=f.as_str() selected=move || format.get() == f>
                                      {label}
                                  </option>
                              })
                              .collect_view()}
                      </select>
                      <button
                          type="button"
                          on:click=move |_| { action.dispatch(SetDefaultPostFormat { format: format.get() }); }
                      >
                          "Save"
                      </button>
                  }
              })}
          </Suspense>
      }
  }
  ```
  (Add `use common::render::PostFormat;` if the extracted file doesn't already
  import it. No `<ActionForm>`, no `<select name="format">`.)
- **Edit** `web/src/profile/api.rs` → update **only the comment** on the
  `#[cfg(test)] mod tests` wire-decode test: it no longer submits via an
  `<ActionForm>`; the endpoint still decodes `format=<token>` over server_fn's
  Url codec whatever the client widget. The assertions stay verbatim.
- **Edit** `end2end/tests/profile.spec.ts` (#498 block): change
  `const FORMAT_SELECT = 'select[name="format"]';` →
  `'select#default-post-format';`; update the comment/title away from
  "ActionForm" to the direct-bind dispatch. Keep `FORMAT_SAVE`, the two
  save-and-reload flips, and every other test unchanged.

**Check (spec criteria 8–10):**

- `rg -n "ActionForm|name=\"format\"" web/src/profile end2end/tests/profile.spec.ts`
  → none (criterion 8).
- `cargo nextest run -p web set_default_post_format_wire_rejects_unknown_token`
  → PASS (unchanged assertions, criterion 9).
- `cargo clippy -p web --target wasm32-unknown-unknown -- -D warnings` → PASS.
- **Behavioral verify (real browser):** `cargo xtask e2e-local profile` → the
  #498 round-trip + #401/#545 flows PASS (criterion 10). If it flakes, re-run
  once (heavy-local flake note) before treating a failure as real.
- `cargo xtask check` clean, then commit:
  `refactor(web/profile): modernize default-post-format control to ADR-0065 direct-bind`.

## Final gate (at ship, not a commit)

`cargo xtask validate` — static + wasm-clippy + coverage + full e2e matrix —
must be green (spec criterion 11). Run it once the three commits land, before
opening the PR.

## Self-review

- Every spec acceptance criterion maps to a task: 1–3,6,7 → Tasks 1–2; 4,5 →
  Task 2 checks; 8,9,10 → Task 3; 11 → Final gate.
- Each task is independently verifiable (compile + a named test/grep/e2e) and
  ends green.
- No task smuggles out-of-scope work; the modernization is the spec's authorized
  cleanup, isolated in its own task.
