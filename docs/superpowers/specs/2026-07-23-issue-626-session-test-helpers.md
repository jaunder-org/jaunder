# Spec — issue #626: consolidate user+session seeding boilerplate into shared test helpers

**Issue:** jaunder-org/jaunder#626 (`dx`, Task). **Blocks:** #325. **Kind:**
Pure test-only refactor. No production code, no behaviour change.

> Revised twice after cold review + user direction. Key decisions now baked in:
> (1) the user fixture is a **`SeedUser` builder** that defaults the common case
> and lets a test override only the axis it varies (username / password /
> display*name / operator) — this is what lets `storage/mod.rs` (previously
> proposed out-of-scope) go through the fixture despite its
> password/display_name/ label variation. (2) `storage/mod.rs` **is in scope**
> for its happy-path user seeding; its label-asserting sessions and error-path
> `create_user` tests stay bespoke because the \_call itself* is what they test.
> (3) The AtomPub helper is a **builder-returning** family. (4) Honest counts +
> per-file acceptance residue.

## Problem

Server integration tests (`server/tests/**`) hand-roll user + session seeding
dozens of times. `.create_session(` has **128 call sites across 17 files**
(heaviest: `web/web_posts.rs` 56, `web/web_account.rs` 14,
`misc/media_handlers.rs` 11, `storage/mod.rs` 10, `atompub/*` 10,
`feed/feed_events_hook.rs` 5). `.create_user(` has ~290 sites, **148 of them in
`storage/mod.rs`**. Almost all pass the same literals and vary only one axis
(usually the username; sometimes the password, display name, operator flag, or
session label). **Five** files already grew private local seeding helpers
(`web_sessions`, `web_site`, `web_backup`, `web_password_reset`, and
`seed_alice` in `atompub/atompub_posts.rs`).

The cost: any signature change to `create_user` / `create_session` is a
~100-site sweep across a dozen unrelated files. #325 (typing the session label
as a `SessionLabel` newtype through `SessionStorage::create_session`) is the
trigger.

## Goal

Factor seeding into a small, **layered** set of shared test fixtures so common
setup is a single call, the five local copies collapse, and a future
seeding-signature change touches a couple dozen sites instead of ~100. Each
specialized helper builds on a more general one; the `create_user` /
`create_session` parse-and-`expect` blocks live in exactly one place each and
are never copy-pasted between helpers.

### On the #325 unblock (honest framing)

This does not reduce the `SessionLabel` sweep to one site — the label-asserting
tests keep explicit `create_session(user_id, "<label>")` calls #325 must still
touch. It routes the ~110 _default-label_ sites through one helper
(`create_session_for`) so the default label literal lives in one shared place.
The primary justification is DX / test quality; the #325 assist is real but
partial.

## Design

### The user fixture — a `SeedUser` builder (in `storage::test_support`)

The one axis that reliably varies is _which_ field a given test cares about:
username always, and occasionally password (auth/duplicate tests), display_name
(profile/relationship tests), or operator (admin-gated tests). A builder
defaults the common case and overrides only what a test varies — so a
non-default-password or display-name test still goes through the fixture instead
of hand-rolling `create_user(&username(..), &password(..), None, false)`:

```rust
/// Fixture for a seeded user. Defaults: password `password123`, no display name,
/// non-operator — override only what a test varies. `seed()` runs the real
/// `UserStorage::create_user` and returns the new `UserId`; it `expect()`s
/// success, so it is happy-path setup only (error-path tests — duplicate
/// username, hash failure — call `create_user` directly and assert the error).
pub struct SeedUser<'a> { /* username, password, display_name, is_operator */ }
impl<'a> SeedUser<'a> {
    pub fn new(username: &'a str) -> Self;         // defaults baked in
    pub fn password(self, p: &'a str) -> Self;
    pub fn display_name(self, d: &'a str) -> Self;
    pub fn operator(self) -> Self;
    pub async fn seed(self, state: &Arc<AppState>) -> UserId;
}
```

Usage (only-override-what-varies; no positional `None`/`false` noise, no
explicit defaults):

```rust
SeedUser::new("alice").seed(state).await
SeedUser::new("alice").password("pw1234567").seed(state).await
SeedUser::new("alice").display_name("Alice").seed(state).await
SeedUser::new("operator").operator().seed(state).await
```

This is the single home of the `create_user` parse block. The existing
`storage::test_support::seed_user(state) -> UserId` becomes a do-nothing alias
for `SeedUser::new("testuser").seed(state)` — so it is **removed, not kept as a
wrapper**. All **~55** callers (the storage-crate unit tests under
`storage/src/**` `#[cfg(test)]`, plus the two in the `test-support` crate's own
tests) are migrated to `SeedUser::new("testuser").seed(state).await`, preserving
the `"testuser"` literal that some downstream lookups assert on.

### Return type for sessions — a named struct

```rust
/// A user seeded together with one authenticated web session. `token` is the raw
/// session token; `cookie()` renders the `session=<token>` header for cookie auth.
pub struct SeededSession { pub user_id: UserId, pub token: RawToken }
impl SeededSession { pub fn cookie(&self) -> String { session_cookie(&self.token) } }
```

Serves cookie-only (`…await.cookie()`), user_id+cookie, and token-auth (atompub
— uses `.token`, no cookie built) call sites by name. (A separate cookie-only
`-> String` helper was considered and rejected: `.cookie()` allocates the same
`String` and the struct is two cheap fields — a second helper just fragments.)

### The session/auth helpers (in `server/tests/helpers/mod.rs`)

Legal because `helpers/mod.rs` already imports from `storage::test_support` and
has the axum request imports. Each is a thin layer:

| Helper                        | Signature                                                             | Built on                                                         | Collapses                                                                                                                                                                                                                                                                                     |
| ----------------------------- | --------------------------------------------------------------------- | ---------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `create_session_for`          | `(state, user_id) -> SeededSession`                                   | `create_session`                                                 | "a session for an existing user"; the one home of the default `"test session"` label.                                                                                                                                                                                                         |
| `create_user_and_session`     | `(state, username) -> SeededSession`                                  | `SeedUser::new(username).seed` + `create_session_for`            | the workhorse — single-user, two-user (twice), inline, atompub token cases.                                                                                                                                                                                                                   |
| `create_operator_and_session` | `(state, username) -> SeededSession`                                  | `SeedUser::new(username).operator().seed` + `create_session_for` | web_site / web_backup operator cases (~25 sites).                                                                                                                                                                                                                                             |
| `atompub_authed`              | `(method, uri, username, token) -> request::Builder`                  | `basic_header`                                                   | the universal AtomPub base: method + uri + `Authorization: Basic`. Callers add extra headers (`If-Match`, `slug`, `Idempotency-Key`, content-type) and finish `.body(...)`. Covers every authenticated atompub shape incl. media (binary body + `image/png` + `slug`) and ETag preconditions. |
| `atompub_xml`                 | `(method, uri, username, token, body: Option<&str>) -> Request<Body>` | `atompub_authed`                                                 | convenience for the dominant text case: `Some(xml)` → `application/atom+xml`; `None` → empty body (GET/DELETE).                                                                                                                                                                               |

A test that needs a session on a _custom-shaped_ user (e.g. the `display_name`
session test) composes them:
`let uid = SeedUser::new("x").display_name("Initial").seed(state).await; let s = create_session_for(state, uid).await;`
— no bespoke seeding block.

### Two single-use combined helpers — refactor in place, delegate

`create_user_with_verified_email` (only `web_password_reset.rs`, 6 uses) and
`seed_user_and_tagged_post` (only `web_tags.rs`, 4 uses): **kept local**, bodies
**refactored to delegate** to the shared fixtures (`create_user_and_session` +
`set_email`; `SeedUser` + tagged post) so their duplicated seeding block is
gone. Hoisting a single-consumer helper to the shared module is relocation, not
reuse. _(If you'd rather hoist them, say so.)_

### Scope of adoption — every test that hand-rolls seeding

Both test surfaces are swept:

**`server/tests/**`** — incl. `storage/mod.rs`: its ~148 `create_user`happy-path setups →`SeedUser`(with`.password(..)`/`.display_name(..)`
where they vary); the largest single concentration of the parse block.

**`storage/src/**` `#[cfg(test)]`unit tests** — the ~55`seed_user(&state)`callers →`SeedUser::new("testuser").seed(state)`(and`seed_user`deleted), plus the ~10 direct`create_user`/`create_session` happy-path setups (`backup.rs`, `post_service.rs`, …) → `SeedUser`.

**Bespoke, stays direct (both surfaces):** error-path `create_user` tests
(duplicate username, hash-failure — `users.rs`, `storage/mod.rs`) that assert
the _error_ (the builder `expect()`s success); and `create_session` calls whose
label is the subject under test (`"Laptop"`, `"test"`, `"alice-1"`/`"bob-1"`,
`"session 1/2"`, `"Test Device"` in `sessions.rs`, and the list/revoke tests) —
a fixture that hides the label would defeat the assertion. Incidental
`"test session"` sessions may use `create_session_for`.

### Verified behaviour-preservation notes

- AtomPub setups (incl. `seed_alice`) use label `"MarsEdit"`;
  `rg '\.label' server/tests/atompub/` is **empty** — no atompub test reads a
  session label back — so routing them through the default `"test session"`
  changes nothing observable.
- `storage::test_support::seed_user`'s signature is unchanged; only its body is
  redirected through `SeedUser`.

### Coverage-gate constraint (ADR-0050)

The line-coverage gate measures test-support code (only CRAP excludes
`**/tests/**`). Every branch of every new fixture must be executed by the suite:
each `SeedUser` override method (`password`, `display_name`, `operator`),
`atompub_xml`'s `Some`/`None` arms, `create_operator_and_session`,
`create_session_for`. All are exercised by real call sites after the sweep — no
speculative/unused builder methods or wrappers may be added.

## Acceptance criteria (observable)

1. **The five local seeding helpers are gone**
   (`web_sessions::create_user_and_session`,
   `web_site`/`web_backup::create_session_cookie`, `atompub_posts::seed_alice`,
   and — per the delegate decision — the two single-use helpers' duplicated
   seeding blocks).
2. **`create_user` parse-block is centralised.**
   `rg 'create_user\(&' server/tests -g '*.rs'` (the raw
   `&username(..)`/`&"x".parse()` form) returns only: the `SeedUser::seed` body;
   the enumerated error-path `create_user` tests in `storage/mod.rs`
   (duplicate/hash-failure) and `backup_fixture.rs`; nothing else. All
   happy-path user seeding across `server/tests/**` (incl. `storage/mod.rs`)
   goes through `SeedUser` (directly or via a session helper).
3. **`create_session` direct calls collapse to an enumerated residue.**
   Post-refactor `rg '\.create_session\(' server/tests -g '*.rs'` returns only:
   `helpers/mod.rs` **1** (`create_session_for` body); and the label-asserting /
   list / revoke subject-under-test sites in `storage/mod.rs`, `web_account.rs`
   (`carol-session`/`dave-session`), `web_sessions.rs` (`mobile` + extras) —
   each enumerated in an appendix with its asserted label. No other file calls
   `create_session` directly (down from 128 across 17 files).
4. **AtomPub Basic-auth request building is centralised.** In `atompub/**` no
   `Request::builder()` chain contains `basic_header(...)` except inside
   `atompub_authed` / `atompub_xml`; verify
   `rg -n 'basic_header' server/tests/atompub/` returns only helper-internal
   sites plus enumerated unauthenticated-negative-test builders. Media-upload
   and `If-Match` requests go through `atompub_authed` + explicit
   `.header(...)`.
5. **`seed_user` is gone.**
   `rg -n 'fn seed_user\b|seed_user\(' storage test-support server -g '*.rs'`
   returns no match (definition removed, all callers migrated to `SeedUser`).
6. **Behaviour preserved.** `cargo xtask validate` green (static + clippy +
   coverage + e2e). No non-test code path changes: edits are confined to
   `#[cfg(test)]` modules and the `test-support`-feature-gated
   `storage/src/test_support.rs`; nothing under `web/src`, `server/src`,
   `common/`, or the non-`cfg(test)` bodies of `storage/src/**` changes.
7. **New fixtures are documented** matching the existing helpers' style; clippy
   (incl. lints already in force) passes.

## Out of scope

- Any change to `create_user` / `create_session` production signatures (that is
  #325, which this assists).
- The `test-support` _crate_'s production seeding logic
  (`test-support/src/lib.rs`'s `create_user` / `seed_posts_for_user`, ADR-0046)
  — only its two `#[cfg(test)]` `seed_user` call sites migrate.
