# Plan — #320: converge the `invites` vertical onto the file-level host/wasm split

**Spec:** `docs/superpowers/specs/2026-07-22-issue-320-web-invites-colocate.md`
(problem/why/decisions live there — this plan is "how"). **Decision record:**
`docs/adr/0070-web-vertical-wasm-only-component-files.md` (as amended by #530,
#527). **For agentic workers:** drive with `jaunder-iterate` (delegate a task to
`jaunder-dispatch` when useful); tick checkboxes in real time.

## Review header

**Goal.** Move the `invites` vertical to the ADR-0070 file-level split —
`mod.rs` (wiring) / `api.rs` (`InviteInfo` DTO + `#[server]` fns) /
`component.rs` (wasm-only UI) — deleting the dead SSR `set_status` vestige and
adding an e2e guard for the policy-gating it leaves behind. No `server.rs`, no
`status.rs`. No behavior change to the invite flows.

**Scope.**

- _In:_ the three `web/src/invites/*` files; deleting the in-component
  `#[cfg(feature="server")]` `set_status` block (keep the "Page not found."
  fallback); repointing `Topbar` to `crate::topbar`; rewiring `pages/mod.rs`; a
  new `invite.spec.ts` policy-gating test; the stale-issue-text fix on #320.
- _Out:_ #330 (App/Router move), #312 (`pages/ui.rs` dissolve), #306 (guard),
  any change to invite behavior / wire contract / `InviteCode` secrecy / mailer
  / storage.

**Tasks.**

1. Extract `InviteInfo` + the two `#[server]` fns into `invites/api.rs`;
   `mod.rs` wires + re-exports them.
2. Cutover: move the UI into wasm-only `invites/component.rs` (delete the
   `set_status` vestige, repoint `Topbar`, fix the doc-comment), delete
   `pages/invites.rs`, rewire the router.
3. Add the `invite.spec.ts` policy-gating guard test + fix the stale line-43
   comment.
4. Full gate + conformance check.
5. Issue hygiene: fix #320's stale "Blocked by #312" text.

**Key risks / decisions.**

- The cutover (task 2) is atomic — component move + `set_status` deletion +
  `pages/invites.rs` delete + router repoint must land in one commit or the
  crate won't compile in between.
- Components are now **wasm-only** ⇒ `wasm-clippy` (`-p web`) is load-bearing;
  host `cargo check` alone won't type-check the UI.
- **Local e2e is reaped here** — the new guard test can't be run locally; it's
  written to match the existing `invite.spec.ts` harness exactly and gates in
  CI. Verify structurally (selectors/seeding/login match the file's patterns).
- No `status.rs`: ADR-0070 §6 found no extractable pure logic
  (`render_invite_row` is `view!`-building; validation lives in
  `forms`/`common`).

## Global constraints

- **Verbatim moves.** `InviteInfo`, the `#[server]` bodies, and the `view!`
  trees move unchanged except the spec-authorized edits: `Topbar` path, deleting
  the `set_status` block, the doc-comment refresh, and cfg placement. No
  redesign of the invite flows.
- **Gate.** Pre-commit hook runs `cargo xtask check` (fmt + clippy + Nix
  coverage/tests, incl. `wasm-clippy`). Run `cargo xtask check` green before
  each commit (`jaunder-commit`). **No `Co-Authored-By` trailer.**
- **e2e:** run `cargo xtask validate --no-e2e` (local e2e VM reaped → CI matrix
  gates the four `{sqlite,postgres}×{chromium,firefox}` combos).
- **ADR-0070 invariants:** `target_arch = "wasm32"` only on `mod` declarations /
  paired `pub use`; zero cfgs inside `component.rs` (**including no
  `#[cfg(feature="server")]`** — the vestige is deleted, not moved); no
  `cov:ignore` / `#[component]`-exemption; no fake host stub (ADR-0055).

---

## Task 1 — extract `InviteInfo` + `#[server]` fns into `invites/api.rs`

**Why:** ADR-0070 (#530): wire types + endpoints live in `api.rs`, `mod.rs` is
wiring. Pure move; keeps `pages/invites.rs` compiling via re-exports.

**Files:**

- **New `web/src/invites/api.rs`:** move, verbatim, from the current
  `web/src/invites/mod.rs`: the `#[cfg(feature = "server")] use { … }` block,
  the ungated `use` lines (`crate::error::WebResult`, `common::email::Email`,
  `common::ids::UserId`, `common::time::UtcInstant`, `leptos::prelude::*`,
  `serde::{Deserialize, Serialize}`), the `InviteInfo` struct + its doc, and
  both `#[server]` fns (`create_invite`, `list_invites`) with docs. Add a `//!`
  doc:
  `//! Invites vertical — API surface: the InviteInfo wire type and the invite #[server] endpoints.`
- **Rewrite `web/src/invites/mod.rs`** to wiring only (this task lands just
  `mod api;` + the `pub use api::{…}` line; the `component` lines arrive in task
  2):

  ```rust
  //! Invites vertical — module wiring (ADR-0070). API surface in `api.rs`, the
  //! wasm-only UI in `component.rs`.
  mod api;

  pub use api::{create_invite, list_invites, CreateInvite, ListInvites, InviteInfo};
  ```

**Verify (pure move):**

- `cargo xtask check --no-test` → green (fmt + host clippy + wasm-clippy).
- `cargo check -p web --all-features` → green.
- `rg 'server\)]' web/src/invites/mod.rs` returns nothing; `pages/invites.rs`
  still imports `crate::invites::{list_invites, CreateInvite, InviteInfo}` and
  resolves.

**Commit:**
`refactor(web/invites): move InviteInfo + #[server] fns into api.rs; mod.rs wiring`

---

## Task 2 — cutover: wasm-only `component.rs`, delete SSR vestige, rewire router

**Why:** co-location + gating, plus removing the dead `set_status` block that
ADR-0070 forbids inside `component.rs`. Atomic (crate won't compile mid-way).

**Files:**

- **New `web/src/invites/component.rs`** (`#[cfg(target_arch="wasm32")]` on the
  `mod` line; **zero cfgs inside**): move `InvitesPage` + `render_invite_row`
  from `pages/invites.rs`, with these edits:
  - Imports: `use crate::topbar::Topbar;` (not `crate::pages::Topbar`); the
    vertical leaves via `use super::{list_invites, CreateInvite, InviteInfo};`;
    keep `crate::error::WebError`, `crate::forms::{Field, ValidatedInput}`,
    `crate::registration::get_registration_policy`, `common::email::Email`,
    `common::registration::RegistrationPolicy`, `leptos::prelude::*`.
  - **Delete the `#[cfg(feature = "server")]` block**
    (`pages/invites.rs:32-39`). The non-invite-only branch becomes simply:
    ```rust
    if policy.await != Ok(RegistrationPolicy::InviteOnly) {
        return view! { <p>"Page not found."</p> }.into_any();
    }
    ```
  - Refresh the `InvitesPage` doc-comment: drop "Returns 404 (via SSR response
    options) when the registration policy is not `invite_only`."; replace with a
    line noting it renders a client-side "Page not found." fallback for
    non-invite-only sites (there is no SSR — ADR-0040/0041/#180).
  - Add `//!` doc:
    `//! Invites vertical — wasm-only UI (ADR-0070): the invite management page.`
- **`web/src/invites/mod.rs`:** add
  `#[cfg(target_arch = "wasm32")] mod component;` and
  `#[cfg(target_arch = "wasm32")] pub use component::InvitesPage;`.
- **Delete `web/src/pages/invites.rs`.**
- **`web/src/pages/mod.rs`:** delete `pub mod invites;` (line 1); move the
  router import out of the `crate::pages::` group to the `crate::` vertical
  group — `use crate::invites::InvitesPage;` (alphabetically between `home` and
  `media`). The `<Route path=StaticSegment("invites") view=InvitesPage />` line
  is unchanged.

**Verify:**

- `cargo xtask check --no-test` → green — **wasm-clippy is the load-bearing
  check** (UI now wasm-only).
- `rg 'crate::pages::Topbar' web/src/invites/` → none;
  `rg -n '#\[cfg' web/src/invites/component.rs` → only `//!`/doc text, no real
  attribute (proves the vestige is gone and no cfg is inside).
- `rg 'ResponseOptions|set_status' web/src/invites/` → none.
- `rg 'mod invites' web/src/pages/mod.rs` → no `pub mod invites;`;
  `web/src/pages/invites.rs` no longer exists.

**Commit:**
`refactor(web/invites): move UI into wasm-only component.rs; drop dead SSR set_status; rewire router`

---

## Task 3 — e2e policy-gating guard + comment fix

**Why:** lock the client-side "Page not found." fallback we kept in task 2 (spec
Decision 4) — currently unverified — and correct the now-stale `404` comment.

**Files:**

- **`end2end/tests/invite.spec.ts`:** add a third test (self-sets `open`, so
  placement is order-independent; the file's `afterAll` already restores
  `open`):

  ```ts
  // Test C — policy guard: on a non-invite-only site the authed /invites page
  // renders the "Page not found." fallback and no create form. Locks the
  // client-side policy-gating (#320 removed the dead SSR set_status 404).
  test("invites page shows not-found fallback when not invite-only", async ({
    page,
  }) => {
    seedConfigViaTool("site.registration_policy", "open");
    const firstNav = slowBrowserFirstNavigationTimeoutMs(test.info(), 15_000);

    await login(page, "testoperator", "testpassword123");
    await goto(page, "/invites", { timeout: firstNav });

    await expect(page.locator('p:has-text("Page not found.")')).toBeVisible({
      timeout: 10_000,
    });
    await expect(page.locator('input[name="recipient_email"]')).toHaveCount(0);
  });
  ```

  (`test`, `expect`, `slowBrowserFirstNavigationTimeoutMs`, `login`, `goto`,
  `seedConfigViaTool` are all already imported at the top of the file.)

- **`end2end/tests/invite.spec.ts:43` comment:** change "(404s unless
  invite_only, which we just set)" → "(shows a 'Page not found.' fallback unless
  invite_only, which we just set)".

**Verify:**

- `cargo xtask check --no-test` → green (includes `tsc` + `prettier` over the
  e2e TS — catches type/format errors in the new test).
- Structural check: the test uses only helpers/selectors already in the file;
  `p:has-text("Page not found.")` targets `InvitesPage`'s `<p>` (the Router
  fallback is a bare text node, no `<p>`), and `input[name="recipient_email"]`
  count-0 confirms the form is absent. **The test itself gates in CI** (local
  e2e VM reaped).

**Commit:**
`test(e2e/invites): guard non-invite-only /invites fallback; fix stale 404 comment`

---

## Task 4 — full gate + conformance check

**Why:** prove the acceptance floor.

**Steps:**

- `cargo xtask validate --no-e2e` → green (static + wasm-clippy + coverage).
  Read `.xtask/last-result.json` `steps[]` if anything is not `ok`.
- Walk the spec's 13 acceptance criteria and confirm each observably (file set =
  `mod.rs`/`api.rs`/`component.rs`, no `server.rs`/`status.rs`; no
  `crate::pages::Topbar`; no `ResponseOptions`/`set_status` in the vertical;
  router repointed; wire structs re-exported; doc-comment refreshed).
- **e2e:** local VM reaped — do not run locally. The three `invite.spec.ts`
  flows (two existing + the new guard) gate in CI; note this in the ship PR
  body.
- `git status --porcelain` after green — stage/commit any fmt residue.

**Commit (if any residue):** `chore(web/invites): gate fixups`

---

## Task 5 — issue hygiene: fix #320's stale "Blocked by #312" text

**Why:** spec Decision 7. #320's body says "Blocked by the prereq (#312)"; the
native graph is the reverse. Not code — `gh issue edit` at ship time.

**Steps:**

- Edit #320's body to strike/correct the "Blocked by the prereq (#312)" line
  (same correction landed on #318). Leave the re-scope notes intact. No commit.

---

## Self-review

- Every spec acceptance criterion maps to a task: 1 (api split, wire re-exports,
  no server.rs), 2 (component gating, vestige delete, Topbar, router, no cfg
  inside, doc-comment), 3 (e2e guard, comment fix), 4 (gate, no-status.rs,
  no-stub, conformance), 5 (issue text).
- Tasks ordered so the crate compiles after each (task 1 keeps
  `pages/invites.rs` alive via re-exports; task 2 is the atomic cutover).
- No task smuggles out-of-scope work (#330/#312/#306 excluded).
- No placeholders: exact files, exact edits, exact `cargo`/`gh` commands.
