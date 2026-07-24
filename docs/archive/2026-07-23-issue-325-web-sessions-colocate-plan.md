# Plan — #325: converge the `sessions` vertical + `SessionLabel` newtype

**Spec:** `docs/superpowers/specs/2026-07-23-issue-325-web-sessions-colocate.md`
(what/why). **For agentic workers:** `jaunder-iterate` drives execution,
delegating a task to `jaunder-dispatch` when useful; tick checkboxes in real
time.

## Review header

**Goal.** Converge the sessions vertical onto the
`mod.rs`/`api.rs`/`component.rs` split (delete `pages/sessions.rs`), introduce a
`SessionLabel` string-newtype and thread it through the web wire _and_ storage,
and modernize both `<ActionForm>`s to ADR-0065 direct-bind.

**Scope.**

- _In:_ `common::SessionLabel`; retype `create_session`/`SessionRecord`/the wire
  DTOs/fns + the two internal callers (login, registration); the file split +
  rewire; modernize the create + revoke forms; update `web_sessions.rs` +
  `atompub.spec.ts`.
- _Out:_ #330, #312 (the `crate::pages::` shim itself), session-auth semantics
  beyond the `label` field, other `SessionRecord` fields. No new ADR.

**Tasks (one line each).**

1. Add `common::session_label::SessionLabel` (DisplayName-parity `StrNewtype`) +
   unit tests.
2. Converge the vertical — mechanical move to `api.rs`/`component.rs`/`mod.rs`,
   rewire `pages/mod.rs`, delete `pages/sessions.rs`. Behavior-preserving
   (forms + `String` label unchanged).
3. Thread `SessionLabel` through the wire + storage + all three `create_session`
   callers; update `web_sessions.rs` (blank-label status, add over-long).
4. Modernize both forms to direct-bind (`Field<SessionLabel>` create, button
   revoke); update `atompub.spec.ts` selectors/comments.

**Key risks / decisions.**

- **Green at every commit.** T2 moves code verbatim (label stays `String`, both
  `<ActionForm>`s intact) so the router repoint + delete land together. T3's
  `create_session` signature change is **atomic** — retyping the `dyn`-trait
  param breaks every caller at once (the 4 production callers —
  `sessions::create_app_password`, login, registration, `server/src/commands.rs`
  CLI — plus the test seed sites), so all land in one commit.
- **#626 shrank the sweep 134 → ~18 sites.** The web suite now funnels through
  the `create_session_for` fixture (built for this in #626 — "the #325 signature
  change touches only here"); T3 updates that one fixture + ~8 direct
  storage-layer sites + a couple web sites + the 4 production callers, via
  `parse_session_label`.
- **Read-side validating decode (accepted).**
  `SessionRecord.label: SessionLabel` re-validates stored labels on read; safe
  because every write path bounds the label ≤200 (the login UA truncation is
  load-bearing). Spec Decision 4.
- **Blank-label rejection moves to decode.** Deleting the server-side check
  means `label=%20%20` is rejected at arg-decode — re-verify
  `create_app_password_rejects_blank_label`'s expected status (500 → likely 400)
  and adjust. Spec 8a.
- **Security-adjacent.** `revoke_session`'s ownership guard is pinned by the
  existing `revoke_session_rejects_session_belonging_to_another_user`; it must
  stay green. Full ship review.
- **wasm-only after T2.** `wasm-clippy` is load-bearing for the moved/modernized
  `SessionsPage` — run it before committing T2 and T4.

## Global constraints

- **No `Co-Authored-By` trailer.**
- **Before each commit:** `cargo xtask check` clean; for server-gated web code
  also `cargo check -p web --all-features --all-targets`.
- **Before T2 & T4 (wasm code):**
  `cargo clippy -p web -p client --features csr --target wasm32-unknown-unknown -- -D warnings -A clippy::too_many_arguments -A unfulfilled_lint_expectations`.
- **Storage tests are dual-backend** (`#[apply(backends)]`, ADR-0053) — the
  `test-backend-pattern` guard fails a bare `#[tokio::test]` that should be
  dual-backend. `SessionLabel`'s sqlx bridge must satisfy the
  `sqlx-newtype-bind` gate.
- `common` gains a module → a `common` change triggers a broad rebuild; expect
  the coverage gate to take a full pass on T1/T3.
- **Import discipline**; `component.rs` reaches endpoints via `super::api`.

---

## Task 1 — `common::session_label::SessionLabel`

**Files:**

- **Create** `common/src/session_label.rs`, modelled on
  `common/src/display_name.rs`:

  ```rust
  use std::str::FromStr;
  use macros::StrNewtype;
  use thiserror::Error;

  /// Maximum session/app-password label length, in Unicode scalars. Generous — a
  /// label is a free-form human tag (device/app name) — while bounding the stored
  /// value and the create form. Chosen ≥ the login UA-label truncation (200) so
  /// the read-side decode of existing rows never fails (#325).
  pub const MAX_SESSION_LABEL_CHARS: usize = 255;

  /// A validated session label: trimmed, non-empty, ≤ `MAX_SESSION_LABEL_CHARS`.
  #[derive(Clone, Debug, PartialEq, Eq, StrNewtype)]
  pub struct SessionLabel(String);

  #[derive(Debug, Error)]
  #[error("session label must be non-empty and at most {MAX_SESSION_LABEL_CHARS} characters")]
  pub struct InvalidSessionLabel;

  impl FromStr for SessionLabel {
      type Err = InvalidSessionLabel;
      fn from_str(s: &str) -> Result<Self, Self::Err> {
          let trimmed = s.trim();
          if trimmed.is_empty() || trimmed.chars().count() > MAX_SESSION_LABEL_CHARS {
              return Err(InvalidSessionLabel);
          }
          Ok(SessionLabel(trimmed.to_owned()))
      }
  }
  ```

  Plus a `#[cfg(test)] mod tests` mirroring DisplayName's: parse/preserve, trim,
  reject empty/whitespace, cap boundary, `Display`, and serde round-trip +
  reject.

- **Edit** `common/src/lib.rs`: add `pub mod session_label;` (alphabetical).

**Check:**

- `cargo nextest run -p common session_label` → PASS.
- `cargo xtask check` clean, commit:
  `feat(common): add SessionLabel validated string-newtype (#325)`.

## Task 2 — Converge the vertical (mechanical move)

**Files:** (label stays `String`; both `<ActionForm>`s move verbatim — this is a
pure re-home, no behavior change.)

- **Create** `web/src/sessions/api.rs`: move `SessionInfo`, `AppPassword`, the
  grouped `#[cfg(feature = "server")]` use-block, and the three `#[server]` fns
  out of `sessions/mod.rs` verbatim.
- **Create** `web/src/sessions/component.rs` (`#[cfg(target_arch = "wasm32")]`
  at the `mod` line): move `SessionsPage` from `pages/sessions.rs`; `Topbar` →
  `crate::topbar`; reach items via
  `use super::api::{list_sessions, CreateAppPassword, RevokeSession, SessionInfo};`.
- **Edit** `web/src/sessions/mod.rs` → wiring: `//!` doc (mirror
  `profile/mod.rs`), `mod api;`,
  `#[cfg(target_arch = "wasm32")] mod component;`, re-exports
  (`pub use api::{list_sessions, create_app_password, revoke_session, CreateAppPassword, RevokeSession, ListSessions, SessionInfo, AppPassword};`
  and `#[cfg(target_arch = "wasm32")] pub use component::SessionsPage;`),
  trimmed to what the registrar + router need.
- **Delete** `web/src/pages/sessions.rs`; **edit** `web/src/pages/mod.rs`: drop
  `pub mod sessions;` (line 1) and `use crate::pages::sessions::SessionsPage;`
  (line 24) → add `use crate::sessions::SessionsPage;` to the `crate::` cluster;
  `/sessions` `<Route>` unchanged.

**Check (spec criteria 1–6):**

- `rg -n "target_arch" web/src/sessions` → only the two `mod.rs` wiring lines;
  `rg -n "cov:ignore" web/src/sessions` → none; `rg "pages::sessions" web/src` →
  none; `pages/sessions.rs` absent.
- `cargo check -p web --all-features --all-targets` → PASS; wasm-clippy → PASS.
- `cargo xtask check` clean, commit:
  `refactor(web/sessions): converge onto api.rs/component.rs/mod.rs; drop pages/sessions.rs`.

## Task 3 — Thread `SessionLabel` through wire + storage + callers

**Files:** (one atomic type change — every `create_session` caller lands here.)

- **`web/src/sessions/api.rs`:** `create_app_password(label: SessionLabel)`
  (drop the manual `trim`/`is_empty` check — the newtype guarantees it);
  `AppPassword.label: SessionLabel`; `SessionInfo.label: SessionLabel`; call
  `sessions.create_session(auth.user_id, &label)`.
  `use common::session_label::SessionLabel;`.
- **`storage/src/sessions.rs`:**
  `create_session(&self, user_id, label: &SessionLabel)` (trait + both impls;
  bind `label.as_ref()`); `SessionRecord.label: SessionLabel`.
- **`storage/src/helpers.rs`:** `SessionRow.label` decodes as `SessionLabel`
  (the StrNewtype sqlx bridge) and `session_record_from_row` carries it through.
- **`web/src/auth/api.rs`** (login): build a `SessionLabel` from the truncated
  UA —
  `let label: SessionLabel = derived.parse().unwrap_or_else(|_| "Unknown device".parse().expect("valid literal"));`
  then `create_session(record.user_id, &label)`.
- **`web/src/registration/api.rs`:**
  `let label: SessionLabel = "Sign-up session".parse().expect("valid literal"); … create_session(user_id, &label)`.
- **`common/src/test_support.rs`:** add
  `pub fn parse_session_label(s: &str) -> SessionLabel` (mirrors the 28 existing
  `parse_*` helpers, e.g. `parse_display_name`) — the convention every
  `#[cfg(test)]` seed site uses to build the newtype.
- **Test call sites (compiler-forced; #626 shrank these from 134 → ~18).** The
  web suite already funnels through
  `server/tests/helpers/mod.rs::create_session_for` (whose own comment says the
  "#325 signature change touches only here"); update that one fixture, then the
  remaining **direct** sites (each passes a `&str` literal → wrap in
  `parse_session_label(..)` or `&"..".parse().unwrap()`):
  - `server/tests/helpers/mod.rs:133` (`create_session_for`, `"test session"`).
  - `server/tests/storage/mod.rs` — ~8 direct storage-layer sites (`"Laptop"`,
    `"test"`, `"alice-1"`/`"alice-2"`/`"bob-1"`, `"session 1"`/`"session 2"`/
    `"test session"`); **and line ~5866 `s.label.as_str()` →
    `s.label.as_ref()`** (`SessionLabel` has `AsRef<str>`/`Deref`, no inherent
    `as_str`).
  - `server/tests/web/web_sessions.rs:20` (`"mobile"`) and
    `server/tests/web/web_account.rs:104,110` (`"carol-session"`,
    `"dave-session"`).
  - `storage/src/sessions.rs` in-crate test-support (lines ~254/275,
    `"Test Device"`).
  - **`web/src/test_support.rs:32`** — a
    `SessionRecord { … label: "test".to_string() … }` literal in the
    `MockSessionStorage` `auth_parts` fixture →
    `label: parse_session_label("test")`. (`MockSessionStorage` only
    `expect_authenticate()`s — no `create_session` matcher — and mockall
    regenerates the trait signature automatically, so no other mock edits.)
    Confirmed complete via
    `rg "\.create_session\(|SessionRecord\s*\{|expect_create_session" -g '*.rs'`;
    re-run before committing.
- **`server/tests/web/web_sessions.rs`:**
  - `create_app_password_mints_labelled_session` / the `s.label == "MarsEdit"`
    and `body.contains("MarsEdit")` assertions stay green (generated
    `PartialEq<&str>` + `Display`).
  - `create_app_password_rejects_blank_label`: run it; if the decode-rejection
    status differs from `INTERNAL_SERVER_ERROR`, update the assertion to the
    observed status (rejection still occurs — that's the invariant).
  - **Add** `create_app_password_rejects_overlong_label` (`label` of
    `MAX_SESSION_LABEL_CHARS + 1` chars → rejected).
  - `revoke_session_rejects_session_belonging_to_another_user` stays green
    (revoke keys on `TokenHash`, untouched).

**Check (spec criteria 7, 8, 8a):**

- `cargo check -p web --all-features --all-targets` → PASS (all three callers
  compile).
- `cargo nextest run -p storage session` and
  `cargo nextest run -p jaunder --test integration web_sessions` (or via
  `cargo xtask check`'s coverage) → PASS on both backends.
- `cargo xtask check` clean (coverage + sqlx-newtype-bind gate), commit:
  `refactor(sessions): type the app-password label as SessionLabel through wire + storage`.

## Task 4 — Modernize both forms + e2e

**Files:**

- **`web/src/sessions/component.rs`:**
  - **create** → a `Field::<SessionLabel>` bound
    `<input id="app-password-label">`; a plain `type="button"` "Create app
    password" that dispatches
    `CreateAppPassword { label: field.parsed().expect("enabled only when valid") }`,
    **disabled-until-valid** (`prop:disabled=move || !field.is_valid()`). Note
    `Field::parsed()` returns `Option<SessionLabel>`, but the wire arg is a
    required (non-`Option`) `SessionLabel`, so the dispatch unwraps under the
    disabled guard — unlike profile's `Option<DisplayName>` field which
    dispatches the `Option` directly. Inline error on touch (mirror
    `profile/component.rs`). Drop the `<ActionForm>` + `<input name="label">`.
  - **revoke** → per row, a plain `type="button" class="j-btn is-danger"` that
    dispatches `RevokeSession { token_hash: s.token_hash.clone() }`. Drop the
    `<ActionForm>` + hidden `<input name="token_hash">`.
  - Keep the `.j-app-passwords` section wrapper and `.j-app-password-token code`
    output (e2e hooks).
- **`end2end/tests/atompub.spec.ts`:** update the two
  `'.j-app-passwords button[type="submit"]'` locators (helper + test) to the
  direct-bind button (`button:has-text("Create app password")`);
  `input[name="label"]` → `#app-password-label`; refresh any "ActionForm/submit"
  comment. `.j-app-password-token code` and the label-appears assertion
  unchanged.

**Check (spec criteria 9–11):**

- `rg -n "ActionForm|name=\"label\"|name=\"token_hash\"" web/src/sessions end2end/tests/atompub.spec.ts`
  → none.
- wasm-clippy → PASS.
- **Behavioral:** `cargo xtask e2e-local atompub` → the mint-app-password +
  AtomPub Basic-auth flow PASS (re-run once on a heavy-local flake before
  treating a failure as real).
- `cargo xtask check` clean, commit:
  `refactor(web/sessions): modernize create/revoke to ADR-0065 direct-bind`.

## Final gate (at ship)

`cargo xtask validate` (full matrix) green — spec criterion 11 — before opening
the PR.

## Self-review

- Spec criteria → tasks: 1–6 → T2; 7 → T1; 8/8a → T3; 9/10 → T4; 11 → final
  gate.
- Each task ends green and is independently verifiable; the atomic
  `create_session` retype is contained in T3.
- No out-of-scope work; the two internal-caller edits are compiler-forced, not
  smuggled scope.
