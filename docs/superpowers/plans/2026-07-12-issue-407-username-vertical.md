# Plan — Issue #407: `Username` vertical (StrNewtype + thread `Username` everywhere)

**Spec:**
[`docs/superpowers/specs/2026-07-12-issue-407-username-vertical.md`](../specs/2026-07-12-issue-407-username-vertical.md)
— read it for the _what/why_, the full site map, and the two approved behavior
decisions. This plan is the _how_: task-by-task, complete Rust, exact `cargo`
commands.

**For agentic workers:** drive execution with **`jaunder-iterate`** (delegate an
individual task to a subagent via **`jaunder-dispatch`** when useful); tick
checkboxes in real time. Commit each task via **`jaunder-commit`** (the
pre-commit hook runs `cargo xtask check`). No `Co-Authored-By` trailer.

---

## Review header

**Goal:** Adopt `#[derive(StrNewtype)]` on `Username` and thread `Username`
through every remaining bare `String`/`&str` username site (host, server, web),
completing the milestone-13 vertical. `common`/`storage` are already typed.

**Scope — in:**

- `common`: derive adoption + full trailer unit tests; the compiler-forced
  `.as_str()` sweep everywhere.
- `host`: the Basic-auth claim (`parse_basic_auth`,
  `Credential.expected_username`).
- `server`: atompub `Path<Username>` + 2 CLI args.
- `web`: DTO/view fields, `#[server]` wire args, component props, internal
  plumbing, and `<ValidatedInput<Username>>` on the two user-typed username
  forms (register, forgot-password).

**Scope — out:** No separable concern warrants a spun-off issue (the vertical is
one cohesive value class, ADR-0063). No new ADR (both decisions apply ADR-0063
§4 / ADR-0065). The storage bind sweep is **in** scope (spec §H) — nothing
mechanical is deferred.

**Tasks (one line each):**

1. `common`: adopt `StrNewtype` on `Username`, extend trailer tests, sweep all
   forced `.as_str()` sites (common + storage + server compare/format).
2. `host`: type the Basic-auth claim —
   `parse_basic_auth → Option<(Username, String)>`,
   `expected_username: Option<Username>`, collapse `basic_username_matches`.
3. `server` atompub: `Path<Username>` across all handlers +
   `require_user_match(&Username)`; malformed path → 400 test.
4. `server` CLI: `UserCreate.username` / `MintAppPassword.username` →
   `Username`.
5. `web` posts slice: DTO fields (`PostResponse`, `TimelinePostSummary`),
   `PostView`, `get_post`/`list_user_posts*` args + internal helpers +
   `PostPage`/`render` plumbing.
6. `web` subscriptions + `current_user`: subscribe/unsubscribe/is-subscribed
   args + `resolve_author`, `current_user() → Option<Username>`,
   `SubscribeButton`, cockpit/`InlineComposer` plumbing.
7. `web` profile slice: `ProfileData`, `PageSeed::{Profile,UserTag}`, profile
   page, `RsdDiscovery`, render-seed threading.
8. `web` register form (ADR-0065): `register(username: Username, …)` +
   `RegisterPage` → `<ValidatedInput<Username>>`.
9. `web` forgot-password form (ADR-0065):
   `request_password_reset(username: Username)` + `ForgotPasswordPage` →
   `<ValidatedInput<Username>>`.

**Key risks/decisions:**

- **Sequencing:** Task 1 both adopts the derive (which _deletes_ the inherent
  `as_str()`) **and** sweeps every forced `.as_str()` site in the same commit —
  those sites all sit on already-`Username` values
  (`common`/`storage`/`auth_user.username`/`post.author_username`), so the
  commit is self-contained and green without any threading. Tasks 2–9 are then
  pure `String → Username` threading, each independently green.
- **Coverage (spec Risk 1) — a non-risk:** the trailer is already exhaustively
  tested in `macros/tests/str_newtype.rs`, and the CRAP-based gate (ADR-0050,
  T=30) can't be moved by the complexity-1 generated impls (CRAP ≈ 2 even
  uncovered). So **no adopter-site trailer tests** are added.
  `field_error::<Username>`/`Field::<Username>` are already exercised by login
  (#414), so Tasks 8–9 add no new uncovered host code.
- **Behavior:** atompub malformed path → 400 (was 403); host claim parse is
  behavior-preserving. Both approved.

---

## Global constraints

- **Backend parity / dual-backend storage tests:** unchanged in this issue —
  `storage` is already typed; no new storage trait methods. Do not touch
  ADR-0019 dialect files.
- **Gate:** before each commit run `cargo xtask check` (fmt + clippy + Nix
  coverage/tests) clean; the pre-commit hook re-runs it. Long/cold runs →
  background (`devtool run -- cargo xtask check`, read `.xtask/run/`).
- **leptosfmt / view! comments** (memory): keep intent comments _outside_
  `view!` macros.
- **web is host + wasm dual-target:** `#[server]` bodies are `cfg(server)`;
  client wiring compiles for both. `common` compiles for wasm — `macros` is a
  build-time proc-macro dep, no wasm runtime footprint.
- Run per-crate checks with `cargo nextest run -p <crate> <filter>`.

---

## Task 1 — `common`: adopt `StrNewtype` on `Username` + trailer tests + forced `.as_str()` sweep

**Files:**

- `common/Cargo.toml` — add dep.
- `common/src/username.rs` — rewrite (derive + tests).
- `storage/src/users.rs`, `storage/src/posts.rs`, `storage/src/postgres/mod.rs`,
  `storage/src/helpers.rs` — `.as_str()` sweep.
- `server/src/atompub/mod.rs`, `server/src/atompub/service.rs`,
  `server/src/atompub/mapping.rs` — `.as_str()` sweep on `auth_user.username` /
  `post.author_username`.

**Interfaces / changes:**

`common/Cargo.toml` — add to `[dependencies]`:

```toml
macros = { path = "../macros" }
```

`common/src/username.rs` — full replacement above the tests:

```rust
use std::str::FromStr;

use macros::StrNewtype;
use thiserror::Error;

/// A validated username matching `[a-z0-9_-]+`.
///
/// Constructed via [`FromStr`] — the single validating/normalizing chokepoint.
/// The rest of the ADR-0063 string-newtype trailer (`Display`, `AsRef<str>`,
/// `Borrow<str>`, `Deref<Target = str>`, owned `String` conversions,
/// `PartialEq<str>`, and the validating serde bridge) is generated by
/// `#[derive(StrNewtype)]`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, StrNewtype)]
pub struct Username(String);

/// Error returned when a string cannot be parsed as a [`Username`].
#[derive(Debug, Error)]
#[error("username must be non-empty and match [a-z0-9_-]+")]
pub struct InvalidUsername;

impl FromStr for Username {
    type Err = InvalidUsername;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.to_lowercase();
        if s.is_empty()
            || !s
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
        {
            return Err(InvalidUsername);
        }
        Ok(Username(s))
    }
}
```

Deletions: the hand-written `TryFrom<String>`, `From<Username> for String`,
inherent `impl Username { as_str }`, `impl fmt::Display`, and
`#[derive(Serialize, Deserialize)]` + `#[serde(try_from, into)]`; drop
`use serde::…` and `use std::fmt` imports.

Tests — **no new trailer tests** (the full generated surface is already
exhaustively tested in `macros/tests/str_newtype.rs`, and the CRAP gate can't be
moved by complexity-1 impls — see header risks). Keep the existing `FromStr`
valid/invalid, `Display`, and serde tests; only rewrite the two compiler-forced
`.as_str()` asserts in `username_normalizes_to_lowercase` (lines ~79, 82):

```rust
#[test]
fn username_normalizes_to_lowercase() {
    let u: Username = "Alice".parse().unwrap();
    assert_eq!(u, "alice"); // was u.as_str() == "alice"
    let u2: Username = "BOB_99".parse().unwrap();
    assert_eq!(u2, "bob_99");
}
```

Sweep (mechanical, compiler-forced — the inherent `as_str()` is gone):

- SQL binds `.bind(username.as_str())` / `.bind(self.author_username.as_str())`
  → `.bind(username.as_ref())` (explicit `&str`). Sites: `users.rs:259,309,399`,
  `posts.rs:79,980,1065,1094,1982,2005,2155,2184`, `postgres/mod.rs:112`.
- `tracing` `fields(username = %username.as_str())` → `%username`. Sites:
  `users.rs:235,283`.
- `server/src/atompub/mod.rs:106` `auth_user.username.as_str() == username` →
  keep for now (`username` here is still the `&str` param until Task 3; use
  `auth_user.username == *username` only after Task 3). **In Task 1, rewrite to
  `auth_user.username == username`** using the new `PartialEq<str>` (param stays
  `&str`).
- `server/src/atompub/service.rs:30` and `mapping.rs:118`
  `let username = …username.as_str();` →
  `let username = &*post.author_username;` (or bind the `Username` and rely on
  `Display`/`Deref` at the `format!` sites).

**Test / verify:**

- `cargo nextest run -p common username` → existing tests (with the rewritten
  `.as_str()` asserts) **PASS**.
- `cargo build --workspace` (or `cargo xtask check --no-test`) → **PASS** proves
  the `.as_str()` sweep is complete (any missed site is a compile error).
- Then `cargo xtask check` clean → commit.

---

## Task 2 — `host`: type the Basic-auth claim

**Files:** `common/src/auth.rs`, `host/src/auth.rs`, `host/src/error.rs` (only
if the error surface shifts — it should not).

**Interfaces:**

- `common::auth::parse_basic_auth(header: &str) -> Option<(Username, String)>` —
  parse the decoded username via `Username::from_str`; a malformed username
  yields `None` (the whole credential is unrecognized), preserving today's
  reject behavior. The password half stays `String`.
- `host::auth::Credential.expected_username: Option<Username>`;
  `resolve_credential` populates `expected_username: Some(username)` from the
  tuple unchanged (now a `Username`).
- `common::auth::basic_username_matches(authenticated: &Username, expected: &Username) -> bool`
  collapses to `authenticated == expected`; update the single call site to pass
  the typed `expected_username`. (If the helper becomes a trivial `==`, inline
  it and delete the fn — confirm the call site during implementation.)

**Test:**

- `common/src/auth.rs` tests: `parse_basic_auth` returns a typed `Username`; a
  malformed username (`"A B:tok"` base64) → `None`. Run
  `cargo nextest run -p common auth` — new assertions PASS.
- `host/src/auth.rs` tests (existing mod at line 85): `resolve_credential` Basic
  path yields `expected_username: Some("alice".parse().unwrap())`. Run
  `cargo nextest run -p host auth`.
- `cargo xtask check` clean → commit.

---

## Task 3 — `server` atompub: `Path<Username>` + `require_user_match(&Username)`

**Files:** `server/src/atompub/{posts.rs,media.rs,rsd.rs,mod.rs}`;
`server/tests/` atompub integration tests.

**Interfaces (Decision 1 — malformed path → 400):**

- `require_user_match(auth_user: &AuthUser, username: &Username) -> Result<(), HandlerError>`
  — body `if auth_user.username == *username`.
- `posts.rs`: `Path(username): Path<Username>` in `collection_get`,
  `collection_post`; `Path((username, post_id)): Path<(Username, i64)>` in
  `member_get`, `member_delete`, `member_put`. URL
  `format!("{base}/atompub/{username}/posts")` uses `Display` unchanged.
- `media.rs`: `Path(username): Path<Username>` in `collection_post`;
  `Path((username, sha, filename)): Path<(Username, String, String)>` in
  `member_get`, `member_delete` (other segments unchanged).
  `media_link_entry(record, base, username: &Username)`.
- `rsd.rs`: `Path(username): Path<Username>` in `rsd_document`.

**Test:**

- Add/adjust a server integration test: an authenticated request to
  `/atompub/{malformed}/posts` (e.g. `A%20B`) now returns **400** (was 403); a
  valid-but-mismatched username still returns **403** (`require_user_match`).
  Grep existing atompub tests for any `403` assertion on a _malformed_ path and
  update it (spec Risk 3).
- `cargo nextest run -p server atompub` — PASS. `cargo xtask check` clean →
  commit.

---

## Task 4 — `server` CLI: `Username` clap args

**Files:** `server/src/cli.rs`; `server/tests/misc/commands.rs`.

**Interfaces:** in `enum Commands`, `UserCreate { … username: Username … }` and
`MintAppPassword { … username: Username … }` — clap parses via `FromStr`, so a
malformed `--username` fails at parse time with a clap arg error (a more-correct
CLI refinement). Downstream `create_user`/mint calls already take `&Username`,
so pass `&username`.

**Test:**

- `commands.rs`: an existing `UserCreate` test still passes; add a case
  asserting a malformed `--username "bad name"` is rejected (clap error /
  non-zero). Rewrite the `.username.as_str() == "…"` assert (commands.rs:419) to
  `== "…"`.
- `cargo nextest run -p server commands` — PASS. `cargo xtask check` clean →
  commit.

---

## Task 5 — `web` posts slice: DTO fields + server-fn args + plumbing

**Files:** `web/src/posts/mod.rs`, `web/src/posts/listing.rs`,
`web/src/render/mod.rs`, `web/src/pages/posts.rs`.

**Interfaces:**

- DTO fields → `Username`: `PostResponse.username`,
  `TimelinePostSummary.username`. `PostView<'a>.username: &'a str` stays a
  borrow, fed from a `Username` via `&*record.username`.
- `#[server]` args → `Username`: `get_post(username: Username, …)`,
  `list_user_posts(username: Username, …)`,
  `list_user_posts_by_tag(username: Username, …)`. Delete the in-body
  `username.parse::<Username>()?` (the arg is already typed). Internal helpers
  `fetch_user_posts(…, username: &Username, …)` /
  `fetch_user_posts_by_tag(…, username: &Username, tag: &Tag?, …)` take
  `&Username`.
- `PostPage` (`pages/posts.rs`): the route `{username}` segment (a `String`) is
  parsed into `Username` on the client at the point it drives the resource —
  `raw_username.strip_prefix('~').and_then(|s| s.parse::<Username>().ok())` →
  `Option<Username>`; skip the fetch when `None` (matches today's
  malformed-route UX — spec Risk 2). Thread `Username` through the resource
  tuple, `SubscribeButton`, and `TagContext::ForUser`.
- `render/mod.rs`: `permalink_article` builds
  `PostView { username: &*post.username }` and
  `TagCtx::ForUser(post.username.clone())`.

**Test:** existing web unit/host tests compile & pass; the test-only
`UserRecord{ username: "…".parse().unwrap() }` sites are unaffected.
`cargo nextest run -p web posts` / `listing` — PASS. `cargo xtask check` clean →
commit.

---

## Task 6 — `web` subscriptions + `current_user()`

**Files:** `web/src/subscriptions/mod.rs`, `web/src/auth/mod.rs`,
`web/src/pages/posts.rs` (`SubscribeButton`), `web/src/pages/cockpit.rs`,
`web/src/pages/ui.rs` (`InlineComposer`).

**Interfaces:**

- `#[server]` args → `Username`: `subscribe_to(author_username: Username)`,
  `unsubscribe_from(author_username: Username)`,
  `is_subscribed_to(author_username: Username)`. Internal
  `resolve_author(users, author_username: &Username, viewer_user_id)` takes
  `&Username`; the `~`-strip/`trim` that fed the old `parse` moves to the
  _caller_ (client) or is dropped since the wire arg is already normalized.
- `current_user() -> WebResult<Option<Username>>` (was `Option<String>`).
- `SubscribeButton(username: Username)`; compares against `current_user()`’s
  `Option<Username>` via `Username == Username`; calls
  `is_subscribed_to(username.clone())`.
- `cockpit.rs`: `username: RwSignal<Option<Username>>`, set from
  `current_user()`; passed to `InlineComposer(username: Username)`.
- `auth::marker::set(&str)` call sites that took a username `String` now pass
  `&*username` / `username.as_ref()`.

**Test:** existing subscription host tests compile & pass.
`cargo nextest run -p web subscriptions` / `auth` — PASS. `cargo xtask check`
clean → commit.

---

## Task 7 — `web` profile slice

**Files:** `web/src/profile/mod.rs`, `web/src/render/mod.rs`,
`web/src/pages/profile.rs`, `web/src/feed_discovery.rs`.

**Interfaces:**

- `ProfileData.username: Username`; populated `username: user.username.clone()`
  (drop `.to_string()`).
- `PageSeed::Profile { username: Username, page }` and
  `PageSeed::UserTag { username: Username, tag, page }`; the render-seed code
  that builds `FeedSurface::User { username }` now receives a `Username`
  directly (no re-parse); title/formatting uses `Display`.
- `profile.rs`: renders `data.username` via `Display` (`{data.username}` —
  already `IntoView` through `to_string`/`Display`).
- `RsdDiscovery(username: Username)`; `rsd_href(username: &Username) -> String`
  (uses `Display`).

**Test:** `cargo nextest run -p web profile` / `render` — PASS.
`cargo xtask check` clean → commit.

---

## Task 8 — `web` register form (ADR-0065)

**Files:** `web/src/auth/mod.rs` (`register`), `web/src/pages/auth.rs`
(`RegisterPage`).

**Interfaces:**

- `register(username: Username, password: String, invite_code: Option<String>) -> WebResult<String>`
  — typed wire arg; delete the in-body
  `username.to_lowercase().parse::<Username>()?`. (`password` stays `String` —
  secret, ADR-0065.)
- `RegisterPage`: replace the raw `RwSignal<String>` + hand-rolled
  `<input name="username" on:input=…to_lowercase()>` with the #414 idiom:

```rust
let username = Field::<Username>::new();
// …
<ValidatedInput<Username>
    label="Username" name="username" autocomplete="username"
    field=username transform=str::to_lowercase />
```

Gate the submit button
`prop:disabled=move || !username.is_valid() /* && other fields */`. On success,
mirror the marker from `username.value.get_untracked()` (as `LoginPage` does).
`name="username"` must match the generated `Register` struct field.

**Test:** `field_error::<Username>` / `Field::<Username>` are already covered
(login). Component rendering is `#[component]` (ADR-0050 exempt) + e2e.
`cargo nextest run -p web auth` — PASS; register e2e flow green
(`cargo xtask e2e …` in ship/validate). `cargo xtask check` clean → commit.

---

## Task 9 — `web` forgot-password form (ADR-0065)

**Files:** `web/src/password_reset/mod.rs` (`request_password_reset`),
`web/src/pages/password_reset.rs` (`ForgotPasswordPage`).

**Interfaces:**

- `request_password_reset(username: Username) -> WebResult<()>` — typed wire
  arg; delete the in-body `username.to_lowercase().parse::<Username>()?`.
- `ForgotPasswordPage`: replace
  `<label>"Username" <input type="text" name="username" /></label>` with:

```rust
let username = Field::<Username>::new();
// …
<ValidatedInput<Username>
    label="Username" name="username" autocomplete="username"
    field=username transform=str::to_lowercase />
```

inside the existing `<ActionForm action=request_action>`; gate the submit button
`prop:disabled=move || !username.is_valid()`. `ResetPasswordPage` is unchanged
(no username field). `ConfirmPasswordReset` unchanged.

**Test:** `cargo nextest run -p web password_reset` — PASS; forgot-password e2e
green. `cargo xtask check` clean → commit.

---

## Final verification (before ship)

- `cargo xtask validate --no-e2e` clean (static + clippy + coverage) — confirms
  no bare `String`/`&str` username remains and the CRAP gate stays green.
- `cargo xtask validate` (full e2e, all 4 backend×browser combos) green —
  auth/register/forgot-password/subscribe/atompub flows.
- Grep sanity: no `\.as_str\(\)` on a `Username` remains; no `username: String`
  / `author_username: String` / `Path<String>` for a username survives.
- Hand to **`jaunder-ship`**.
