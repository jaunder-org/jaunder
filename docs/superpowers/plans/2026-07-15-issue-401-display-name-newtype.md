# Plan — issue #401: `DisplayName` newtype

Spec: `docs/superpowers/specs/2026-07-15-issue-401-display-name-newtype.md`.
Each task is one gate-green commit (`cargo xtask check`). No task leaves the
workspace un-buildable; the read/create path and the write path are threaded
separately (they are not compile-coupled), so no throwaway scaffolding is
needed. Direct precedents: **`Email`** (#397, `Option<Email>` in `UserRecord` +
strict `build_user_record` parse) for storage; **`slug_override`** (#408,
`Option<Slug>` wire arg + `Field::optional` + `.dispatch`) for web.

## Task 1 — `common::display_name::DisplayName`

- [x] Add `common/src/display_name.rs`: `MAX_DISPLAY_NAME_CHARS = 255`;
      `#[derive(Clone, Debug, PartialEq, Eq, StrNewtype)] struct DisplayName(String)`;
      `InvalidDisplayName` (`#[derive(Debug, Error)]`, message interpolates the
      const); hand-written `FromStr` (trim → reject empty → reject `> MAX` via
      `chars().count()` → preserve casing). Mirror `slug.rs` (bound) +
      `tag.rs::TagLabel` (trim/preserve-case).
- [x] Register `pub mod display_name;` in `common/src/lib.rs` (alphabetical,
      after `backup`).
- [x] Inline `#[cfg(test)] mod tests`: parse valid; reject
      empty/whitespace-only/over-255 (boundary: 255 ok, 256 rejected);
      `to_string`/`Display`;
      `..._serde_serializes_as_plain_string_and_validates_on_deserialize`
      (pattern from `username.rs`/`slug.rs`).
- [x] **No** `validation_from!` entry in `host/src/error.rs` (typed wire arg ⇒
      no body parse ⇒ dead code).
- Gate: `cargo xtask check` (workspace still green — nothing consumes it yet).

## Task 2 — thread `DisplayName` through the read/create path

Everything reachable from `UserRecord.display_name` + `create_user`. Leaves
`ProfileUpdate`/`update_profile` **unchanged** (still `Option<&str>`/`String`),
so it compiles and gates green on its own.

- [x] `storage/src/users.rs`: `UserRecord.display_name: Option<DisplayName>`;
      `create_user(..., display_name: Option<&'a DisplayName>, ...)` on the
      trait and the generic impl (`.bind(display_name.map(|d| &**d))` — bind the
      `&str` via `Deref`).
- [x] `storage/src/helpers.rs::build_user_record`: parse the `display_name`
      column strictly, exactly like `email`
      (`.map(|s| s.parse().map_err(|e| sqlx::Error::Decode(Box::new(e)))).transpose()?`).
- [x] Backends: `storage/src/postgres/mod.rs` + `storage/src/sqlite/mod.rs`
      `create_user` signatures/bindings; `storage/src/atomic.rs` wrapper
      forward. No SQL/migration change (column stays `TEXT`).
- [x] CLI: `server/src/cli.rs` `UserCreate.display_name: Option<DisplayName>`
      (clap parses via `FromStr`); `server/src/commands.rs` +
      `server/src/main.rs` pass `.as_ref()` (seed paths stay `None`).
- [x] `test-support/src/lib.rs` + `src/main.rs`: seed CLI's `--display-name` arg
      is typed `Option<DisplayName>` (clap `FromStr`); the lib `create_user`
      helper forwards `Option<&DisplayName>` with no internal parse.
- [x] Web read side: `web/src/profile/mod.rs`
      `ProfileData.display_name:     Option<DisplayName>`; `get_profile` moves
      `user.display_name` directly (no parse). Import `DisplayName` (ungated —
      DTO built on both targets).
- [x] Update compile-forced tests: `server/tests/storage/mod.rs`,
      `server/tests/misc/backup_fixture.rs`, and any `web_account.rs`
      `get_profile` assertions reading `profile.display_name` (use
      `Some("…".parse().unwrap())` or the `PartialEq<str>` on the inner value).
- Gate: `cargo xtask check`.

## Task 3 — write path: typed wire arg + ADR-0065 client validation

- [x] `storage/src/users.rs`:
      `ProfileUpdate.display_name: Option<&'a DisplayName>`; the
      `update_profile` SQL binding maps to `&str` (`.map(|d| &**d)`).
- [x] `web/src/profile/mod.rs::update_profile`: wire arg
      `display_name: Option<DisplayName>`; **drop** `common::text::non_empty`
      for display name (empty→`None` handled client-side); pass
      `display_name.as_ref()` into `ProfileUpdate`. `bio` unchanged (keeps
      `non_empty`).
- [x] `web/src/pages/profile.rs`: convert the profile `<ActionForm>` to a
      `.dispatch` form (mirror `slug_override`):
      `Field::<DisplayName>::optional_prefilled(&existing)`, **direct-bind** the
      existing `<input name="display_name">` (`prop:value`/`on:input` →
      `error_for`/`on:blur` → `touch()` + touched-gated inline error), `bio` in
      an `RwSignal<String>` bound to its `<textarea>`, submit `on:click` →
      `update_action.dispatch(UpdateProfile { display_name: dn_field.parsed(),     bio: bio_sig.get() })`.
      Submit `prop:disabled=move || !dn_field.is_valid()` (blank stays valid ⇒
      clearing works; over-long gates).
- [x] `web/src/forms.rs` tests: add a `field_error::<DisplayName>` case (valid →
      `None`; over-long → the newtype's message).
- [x] Server integration tests (`server/tests/web/web_account.rs`): for the
      invalid/over-long `update_profile` case assert `assert_ne!(status, OK)` +
      the store side-effect and **drop** any `body.contains("<message>")`
      (precedent: `web_auth.rs::register_invalid_username_returns_error`). Keep
      the happy-path + clear-to-`None` assertions.
- Gate: `cargo xtask check`.

## Task 4 — e2e

- [ ] Profile-page e2e (extend the existing account/profile spec): a valid
      display name submits and persists; an over-long entry shows the inline
      client error and gates submit. Selector unchanged
      (`input[name="display_name"]`).
- Gate: `cargo xtask e2e sqlite chromium` (representative combo); full
  `cargo xtask validate` at ship.

## Ship

- [ ] `cargo xtask validate` clean; whole-branch `/code-review` (Standards +
      Spec) + a cold blind review; `jaunder-ship` (archive planning docs, PR
      referencing #401, release the project item to **Done**).

## Notes / risks

- **No new ADR** — adopts ADR-0063 + ADR-0065.
- **Legacy read**: a stored `display_name` > 255 chars would fail
  `build_user_record` (strict read). Accepted per the design decision (255 is
  generous). If any deployment is known to hold longer values, widen the bound
  before Task 1 — do not add a lenient read path.
- **`ProfileData.email` counter-precedent**: stays `Option<String>` (older thin
  style). We type `display_name`; leaving `email` as-is is intentional (out of
  scope).
