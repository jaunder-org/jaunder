# Spec — Issue #407: `Username` vertical (StrNewtype + thread `Username` everywhere)

- Issue: [#407](https://github.com/jaunder-org/jaunder/issues/407) — part of the
  #404 umbrella, milestone #13 (Domain-value type safety).
- Blocked by: #403 (StrNewtype/IdNewtype derives — **CLOSED**) and #414
  (client-side validation pattern, ADR-0065 — **CLOSED**). Both resolved;
  unblocked.
- Governing ADRs: **ADR-0063** (newtype convention: generated trailer, boundary
  rule §4) and **ADR-0065** (typed `#[server]` wire args + client-side
  pre-validation).

## Goal

Complete the end-to-end `Username` vertical: adopt `#[derive(StrNewtype)]` on
`Username`, and replace every remaining bare `String`/`&str` that carries a
username value with `Username` (or `&Username`, or `&str`-via-`Deref` where a
read-only borrow is idiomatic), storage-outward, per ADR-0063 §4 (parse at the
outermost boundary, hold the newtype inward). No wire/behavior change except the
two explicitly-approved edge refinements below.

## Current state (from the site survey)

**`common` and `storage` are already fully migrated to `Username`.** The
remaining surface is:

- **`common/src/username.rs`** — still hand-writes the whole trailer + serde
  bridge; must adopt the derive.
- **`web`** — 5 DTO/view fields, ~8 `#[server]` args + `current_user()`'s
  return, 3 component props, and internal `&str`/`String` plumbing still carry a
  username as a bare string.
- **`server`** — every atompub handler extracts `Path<String>` for `{username}`;
  two clap args are `username: String`.
- **`host`** — one field, `Credential.expected_username: Option<String>` (the
  Basic-auth claim).

### What `#[derive(StrNewtype)]` generates (so we know what to delete)

`Display`, `AsRef<str>`, `Borrow<str>`, `Deref<Target = str>`, `TryFrom<String>`
(via `FromStr`), `From<Self> for String`, `PartialEq<str>`, `PartialEq<&str>`,
and direct `Serialize`/`Deserialize` impls (serialize borrows; deserialize
routes through `FromStr`). It does **not** generate `FromStr`, the std
`#[derive]`s, or any inherent `as_str()`.

### The #414 reference pattern (`login`, already shipped)

`web/src/forms.rs` provides the shared chokepoint:
`field_error<T>(&str) -> Option<String>` (both-target, host-tested), `Field<T>`
(parent-owned live value + validity, signal-only), and
`#[component] ValidatedInput<T>`. `web/src/pages/auth.rs::LoginPage` is the
exemplar: `Field::<Username>::new()`,
`<ValidatedInput<Username> name="username" transform=str::to_lowercase/>`,
submit gated `disable-until-valid`, and `login(username: Username, …)` as the
typed wire arg.

## Approved decisions (design interview)

1. **atompub handlers take `Path<Username>`** (not `Path<String>`). The
   `{username}` path segment is compared to the authenticated user
   (`require_user_match` → 403) and used for URL formatting; it is never a
   lookup key. Typing it parses at the HTTP boundary per ADR-0063 §4. **Accepted
   behavior refinement:** a _malformed_ path username now returns **400 Bad
   Request** (axum deserialize reject) instead of 403 Forbidden. Legitimate
   clients are unaffected; a valid-but-mismatched username still returns 403 via
   `require_user_match`. This is the only intentional behavior change in the
   issue.

2. **Host `Credential.expected_username` becomes `Option<Username>`.**
   `parse_basic_auth` changes to return `Option<(Username, String)>`, parsing
   the claimed username at the header-decode boundary. Behavior-preserving: a
   malformed claim is rejected either way (previously via a failed match, now
   via a `None` credential). `basic_username_matches` collapses to a plain
   `Username == Username`.

## Scope — sites to convert (grouped by layer)

### A. Adopt the derive (`common`)

- Add the `macros` crate as a dependency of `common`.
- `Username`: `#[derive(Clone, Debug, PartialEq, Eq, Hash, StrNewtype)]`; keep
  `FromStr` and `InvalidUsername` verbatim. Delete the hand-written
  `TryFrom<String>`, `From<Username> for String`, inherent `as_str()`,
  `Display`, and the `#[derive(Serialize, Deserialize)]` +
  `#[serde(try_from, into)]` bridge. Drop the now-unused `serde`/`fmt` imports.
- Rewrite the existing `.as_str()` asserts (the inherent method is deleted) to
  `== "lit"` / `.as_ref()`. **No new trailer tests** — the full generated
  surface is already exhaustively tested in `macros/tests/str_newtype.rs`
  (against a `Code(String)` fixture with the same lowercasing/rejecting
  `FromStr`), and per Risk 1 the trivial generated impls can't move the CRAP
  gate. Keep the existing `FromStr`/`Display`/serde tests.

### B. host

- `Credential.expected_username: Option<Username>`;
  `common::auth::parse_basic_auth` → `Option<(Username, String)>`;
  `basic_username_matches` simplified to `==`. Update the `error.rs` mappings if
  the fallible parse changes the error surface (it should not — a bad claim
  yields `None`, not an error).

### C. server

- **atompub** (`Path<String>` → `Path<Username>`, per Decision 1): `posts.rs`
  `collection_get`, `member_get`, `member_delete`, `collection_post`,
  `member_put`; `media.rs` `collection_post`, `member_get`, `member_delete`;
  `rsd.rs` `rsd_document`. `require_user_match(auth_user, username: &Username)`
  compares `Username == Username`; URL `format!`s use `Display`. `member_*`
  folds via `owned_post`. Other path segments (sha, filename, post_id) are
  unchanged.
- **CLI** (`server/src/cli.rs`): `UserCreate.username` and
  `MintAppPassword.username` become `username: Username` (clap parses via
  `FromStr`; malformed → clap arg error, a more-correct refinement of the CLI
  surface).

### D. web — `#[server]` wire args (typed `Username`; drop the internal `parse`)

- `auth::register(username: Username, …)`,
  `current_user() -> WebResult<Option<Username>>`.
- `password_reset::request_password_reset(username: Username)`.
- `subscriptions::{subscribe_to, unsubscribe_from, is_subscribed_to}(author_username: Username)`
  and the internal `resolve_author(…, author_username: &Username, …)` (the
  `~`-strip/trim moves client-side or is dropped — the wire arg is
  already-typed).
- `posts::get_post(username: Username, …)`,
  `posts::listing::{list_user_posts, list_user_posts_by_tag}(username: Username, …)`
  and their internal helpers `fetch_user_posts(…, username: &Username, …)` /
  `fetch_user_posts_by_tag`.

### E. web — DTO / view fields

- `posts/listing.rs::TimelinePostSummary.username`,
  `posts/mod.rs::PostResponse.username`, `profile/mod.rs::ProfileData.username`,
  `render/mod.rs::PageSeed::{Profile,UserTag}.username` → `Username`.
  `render/mod.rs::PostView<'a>.username: &'a str` stays a borrow (fed from a
  `Username` via `Deref`).

### F. web — component props + internal plumbing

- `pages/posts.rs::SubscribeButton(username: Username)`,
  `pages/ui.rs::InlineComposer(username: Username)`,
  `feed_discovery.rs::RsdDiscovery(username: Username)` (+
  `rsd_href(&Username)`).
- Internal `Option<String>`/`String` username plumbing in `pages/posts.rs`
  (PostPage resource tuple), `pages/cockpit.rs` (`RwSignal<Option<Username>>`),
  and `render/mod.rs` threading — typed through as `Username`.

### G. Client-side pre-validation (ADR-0065) — only where the user _types_ a username

- **`RegisterPage`** (`pages/auth.rs`) — currently a raw `RwSignal<String>` +
  hand-rolled `<input>`. Rewrite to `Field::<Username>::new()` +
  `<ValidatedInput<Username> name="username" transform=str::to_lowercase/>`,
  submit gated on `is_valid()`.
- **Password-reset request page** — same treatment for its username field.
- **Login** — already done (#414); no change.
- **Programmatically-sourced usernames** (subscribe button, route-driven
  `get_post` / `list_user_posts`, profile) are **not** user-typed, so they get
  **no** `ValidatedInput`. The client parses the already-valid profile/route
  username into `Username` at the point it calls the server fn (route params:
  parse the `~`-stripped segment; skip the fetch when it fails to parse). Per
  ADR-0065 the client-validation requirement is about typed inputs; these carry
  known-valid values.

### H. `.as_str()` sweep (compiler-forced — the inherent method is deleted)

Deleting the inherent `as_str()` makes **every** `.as_str()` call on a
`Username` a compile error, so this sweep is mandatory across all crates, not
optional. Rewrite each to the idiomatic form the generated trailer now provides:

- **SQL binds** (`storage/src/users.rs`, `posts.rs`, `postgres/mod.rs`) →
  `.bind(x.as_ref())` (explicit `AsRef<str>` → `&str`; works for the `&Username`
  params).
- **`tracing` fields** (`fields(username = %username.as_str())`) → `%username`
  (the generated `Display`).
- **Comparisons / URL formatting** (`atompub/mod.rs`, `service.rs`,
  `mapping.rs`; test asserts) → `== "lit"` (`PartialEq<str>`), `&x`, or
  `Display` interpolation.

All of `storage`'s bind/tracing sites are in scope:
`users.rs:235,259,283,309,399`, `posts.rs:79,980,1065,1094,1982,2005,2155,2184`,
`postgres/mod.rs:112`, plus the `server` and test-assert sites.

## Acceptance

- `Username` derives `StrNewtype`; no hand-written trailer or `#[serde]` bridge
  remains; `FromStr` unchanged; `common` depends on `macros`.
- No bare `String`/`&str` remains for a username value in storage
  records/signatures, host, server, web DTOs, internal web, or component props
  (per the boundary policy).
- User-typed username forms (register, password-reset) use
  `<ValidatedInput<Username>>` with disable-until-valid, matching #414.
- `cargo xtask validate --no-e2e` clean; e2e green for the affected
  auth/subscribe/atompub flows.
- The only behavior change is atompub malformed-path 403 → 400 (Decision 1).

## Risks / notes

1. **Coverage of the generated trailer — a non-risk, kept only as a note.**
   `Username` is the first in-tree `StrNewtype` adopter, so its generated impls
   are attributed to `common/src/username.rs`. But (a) the trailer's _behavior_
   is already exhaustively tested in `macros/tests/str_newtype.rs`, and (b) the
   gate is CRAP-based (ADR-0050, T=30) and every generated impl is
   cyclomatic-complexity-1 → CRAP ≈ 2 even fully uncovered. Today's hand-written
   `TryFrom`/`From` have no explicit unit test yet `common` passes the gate,
   confirming this. **No adopter-site trailer tests are needed.**
2. **Route-driven typed args change client control flow.** `get_post` /
   `list_user_posts*` move parse-on-entry from the server body to a client-side
   parse of the route segment. Confirm the malformed-route path (no fetch /
   404-style page) matches today's UX (today: server returns
   `WebError::Validation`). This is internal web behavior, not a wire change.
3. **atompub 403 → 400.** Confirm no unit/e2e test asserts 403 for a _malformed_
   (vs mismatched) atompub path username; update any that do.
4. **`macros` dep in a wasm-compiled crate.** `common` compiles for wasm (web
   depends on it all-target); `macros` is a build-time proc-macro crate, so the
   dependency is host-only at build time — no wasm runtime footprint (ADR-0062).

## Out of scope / follow-ups

- No separable concern warrants a spun-off issue: the vertical is one cohesive
  value-class change (ADR-0063 "each value class is its own reviewable change").
  We finish the job in one pass — including the full `.as_str()` sweep (§H) —
  rather than leaving mechanical, easy-to-review tails behind.
- Absorbs the `Username` half of #14 (closed).
