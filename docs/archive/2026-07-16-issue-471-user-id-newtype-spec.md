# Spec — #471: `UserId` newtype

- Issue: [#471](https://github.com/jaunder-org/jaunder/issues/471) (sub-issue of
  the umbrella [#457](https://github.com/jaunder-org/jaunder/issues/457))
- Milestone: Domain-value type safety (newtypes)
- Governing ADR: [ADR-0063](../../adr/0063-domain-value-newtype-convention.md)
  (numeric-ID trailer, §"Numeric IDs")
- Date: 2026-07-16

## Problem

A user id crosses the codebase as a bare `i64` through `common`, `storage`,
`server`, and `web`. Any `i64` can be passed where a user id is expected, and at
a call site a `user_id` is indistinguishable from a `post_id`, `audience_id`, or
any other integer — the `tag_post(post_id, …)`-accepts-a-`user_id` transposition
class. Per ADR-0063 §1 the value qualifies on the **transposition** axis. No
invariant, no security surface — the sole win is turning ID mix-ups into compile
errors.

`UserId` is the **first** of the umbrella's eight ID newtypes to land, so it
also **establishes the shared home and pattern** (file layout, sqlx-boundary
conversion, pervasive adoption) that #472–#478 follow.

## Decision

Introduce `UserId` per ADR-0063's numeric trailer and thread it through every
Rust site that carries a user id. Behavior and wire shapes are unchanged; this
is a `refactor:` (type-only) change.

### The type — `common::ids`

```rust
// common/src/ids.rs  (new module — the shared home for ALL eight ID newtypes)
use macros::IdNewtype;   // the `macros` crate (ADR-0062), as `username.rs` does for StrNewtype

/// A user's row id. Newtyped so it can't be transposed with another `i64` id
/// (post, audience, subscription, …) at a call site.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, IdNewtype)]
pub struct UserId(i64);
```

- Lives in **`common`**, in a **single `common/src/ids.rs` module** that will
  house all eight ID newtypes (#472–#478 append their type here), declared
  `pub mod ids;` in `common/src/lib.rs`. `common` exposes no `pub use`
  re-exports — consumers use the full module path, so the type is
  **`common::ids::UserId`**. `common` is namable by client wasm, so web DTOs can
  adopt the type.
- **Home decision (pattern-setter for #472–#478):** the string newtypes each
  earn their own file because they carry a hand-written `FromStr`, an error
  type, and validation tests (~80 lines each). An `IdNewtype` is a doc-comment +
  `struct X(i64);` + derive (~4 lines) with none of that, so one `ids.rs`
  grouping is more navigable than eight near-identical stub files. This **amends
  the umbrella #457 convention** (which said "one file per type"); #457 is
  updated to match.
- `IdNewtype` (ADR-0062 `macros` crate) supplies `From<i64>`,
  `From<Self> for i64`, `Display`, and a **transparent-i64 serde bridge** (wire
  form stays a bare integer).
- **No `Ord`** — no user id is a sort/map key today (verified: no
  `BTreeMap<_, …>` keyed on it, no `.sort()`); add it only if a site needs it.
  `Hash` is kept (IDs plausibly seed `HashSet`/`HashMap`).
- No hand-written `FromStr`, no validation — an id has no value invariant.

### sqlx boundary — convert at the edge

`IdNewtype` derives **no** sqlx traits (`Type`/`Encode`/`Decode`), and the
storage layer is generic over `DB: Backend` with explicit
`for<'q> i64: Encode<'q, DB> + Type<DB>` bounds. So `UserId` crosses the DB
boundary by **conversion, not trait impls** (the established pattern; #438 may
later add a transparent bridge that removes these):

- **Writes:** `.bind(i64::from(user_id))` at each bind site (the `i64: Encode`
  bounds are unchanged — we still bind an `i64`).
- **Reads:** keep decoding a raw `i64` in the `query_scalar::<_, i64>` /
  `query_as` tuple, then **wrap** `UserId::from(raw)` inside the
  `build_*_record` helpers in `storage/src/helpers.rs` — the single per-record
  chokepoint, mirroring how string columns are `.parse()`d there today.
  `RETURNING user_id` scalars wrap at the call site.

SQL column names (`WHERE user_id = $1`, `RETURNING user_id`) are **unchanged** —
the newtype is a Rust-side concern only; the schema and wire bytes are
untouched.

### Pervasive adoption

Every Rust site that carries a user id adopts `UserId` — storage
records/traits/impls, `#[server]` args & returns, web DTO fields,
`common::visibility` viewer constructors, and the values that fold in:
**`author_user_id`** (audiences/subscriptions — the audience author's user id)
and **`MediaRecord.user_id`**. The transparent-i64 serde bridge keeps every
serialized shape identical, so no e2e or wire change. No `parse().expect()` at
the web boundary.

**Carve-out:** `ViewerIdentity::Channel.subscriber_ref` stays a `String` — it is
a _polymorphic_ value (a stringified user id in one arm, an external ref in
another; ADR-0020 / ADR-0063 model it as an enum-with-string, not a `UserId`).
Only the numeric `user_id` parameters of `visibility::local(...)` /
`account_viewer(...)` become `UserId`.

## Scope (layers)

The **concrete ~200-site edit-map is the plan's completeness surface** (there is
no enforcement gate, and — see AC2 — a single grep can't catch every site). The
layers:

1. **common** — define `UserId` in `ids.rs`; in `visibility.rs`, **all** user-id
   params and returns: `local()`/`account_viewer()` params,
   `viewer_user_id() -> Option<UserId>`,
   `SubscriptionPolicy::initial_status`/`OpenSubscriptionPolicy::initial_status`'s
   `author_user_id`. (`subscriber_ref` stays `String`;
   `AudienceTarget::Named(i64)` is an audience id — leave both.)
2. **storage** — record/input structs with a user-id field (`UserRecord`,
   `SessionRecord`, `PostRecord`, `PostRevisionRecord`, `MediaRecord`,
   `CreatePostInput`, `RenderedPostContent`/`PostCreation` `user_id`,
   `RenderedPostUpdate`/`PostUpdate` `editor_user_id`, and
   **`InviteRecord.used_by: Option<UserId>`** — the redeemer's user id); object
   traits **and** the `Backend`-generic dispatch traits; impls (`.bind`/decode);
   the `build_*_record` helpers + their row-tuple types
   (`UserRow`/`SessionRow`/`InviteRow`/ `PostRow`/`MediaRow` user-id positions);
   both backend dirs (`sqlite/`, `postgres/`). **User-id return types become
   `UserId`:** `create_user`, `create_user_with_invite` (`atomic.rs`),
   `use_password_reset`/`confirm_password_reset`, `use_email_verification`
   (`-> (UserId, Email)`). `author_user_id` (audiences/subscriptions) and
   `editor_user_id` (posts) are user ids and become `UserId`.
3. **server** — `atompub/*`, `media.rs`
   (`ProxyParams.user_id`)/`media_manager.rs`, `commands.rs` (`{user_id}`
   Display → `i64::from(...)`).
4. **web** — DTO fields (`AuthUser.user_id`, …), `#[server]` fn params/returns
   (`viewer_user_id: Option<UserId>`, `resolve_author() -> Result<UserId, _>`),
   page components, `viewer.rs`; `auth.user_id.to_string()` →
   `i64::from(auth.user_id).to_string()`.
5. **tests** — construct user ids via `UserId::from(n)` directly (an infallible
   wrap — there is no fallible-parse boilerplate to centralize, unlike the
   `parse_*` string helpers, and an unused thin wrapper trips the line-coverage
   gate); `storage::test_support::seed_user() -> UserId` remains the real
   seeding helper. `assert_eq!`/`==` comparisons need only the derived
   `PartialEq` — no edit.

**Do not over-reach** (nearby `i64`s in touched files that are _not_ user ids):
`AudienceSelection.named: Vec<i64>` (audience ids, #475),
`list_members() -> Vec<i64>` and `SubscriptionRecord.subscription_id`
(subscription ids, #476), `post_id_for_idempotency_key`'s
`query_scalar::<_, i64>` (a post id), media usage-count scalars (a `SUM`). Type
only user ids.

## Acceptance criteria (observable)

- **AC1** `common::ids::UserId` exists (in `common/src/ids.rs`, declared
  `pub mod ids;` in `common/src/lib.rs` — **no `pub use` re-export**, per the
  crate's convention; the type is named by its full path), derives
  `Copy, Clone, Debug, PartialEq, Eq, Hash, IdNewtype`, and has no hand-written
  `FromStr`.
- **AC2** No storage record/input field, storage trait signature (object or
  dispatch), `#[server]` function signature, or web DTO field that holds a user
  id is typed `i64` — each is `UserId`, **including differently-named and
  compound sites**: `author_user_id`, `editor_user_id`, `MediaRecord.user_id`,
  `InviteRecord.used_by: Option<UserId>`, `viewer_user_id: Option<UserId>`, and
  user-id **return types** (`create_user`, `create_user_with_invite`,
  `use_password_reset`, `confirm_password_reset`, `use_email_verification`,
  `resolve_author`). **Completeness surface = the plan's edit-map checklist**,
  not a single grep. Supplementary grep (over touched files only) must cover
  `user_id`/`author_user_id`/`editor_user_id`/`used_by`/`viewer_user_id` **and**
  `Result<i64`/`Option<i64>`/`(i64,` return/tuple forms; SQL string literals
  excepted.
- **AC3** Wire/serialized shapes are byte-identical — a `UserId` serializes as a
  bare integer; all existing e2e and serialization tests pass unchanged.
- **AC4** Both SQLite and Postgres impls compile and pass; no migration is added
  (schema unchanged).
- **AC5** `subscriber_ref` remains `String` (carve-out honored).
- **AC6** `cargo xtask validate --no-e2e` is clean (fmt, clippy incl. no new
  `unwrap`/`expect` in production, coverage gate), and the e2e suite passes.

## Tests

- `common`: unit test the `UserId` derive — `From<i64>`/`Into<i64>` round-trip,
  `Display`, and a serde round-trip proving the wire form is a bare integer
  (`serde_json::to_string(&UserId::from(42)) == "42"`).
- Existing storage/server/web tests: update construction/comparison sites to
  `UserId` (behavior unchanged). No new behavioral tests — this is a type-only
  refactor, and the wire-invariance test above guards the one observable risk.

## Non-goals

- The other seven ID classes (#472–#478) — `PostId`, `TagId`, etc. stay bare
  `i64` here; a signature that mixes `user_id` and (say) `post_id` types only
  the `user_id`.
- A transparent sqlx bridge for the newtype (#438) — out of scope; use boundary
  conversion.
- Any schema/wire/behavior change.

## Risks

- **Concurrent overlap with #458** (session credential types) in `sessions.rs` —
  both touch `SessionRecord`/`create_session`. No hard blocker; rebase whichever
  lands second, touching only the `user_id` field/param here.
- **Scale** — the largest of the eight tracks (~50 files). The compiler
  enumerates every rippled site once the record fields and trait signatures
  flip; lean on `cargo check`. A missed _tightening_ (a local helper left `i64`
  that still compiles) is caught by the AC2 grep, not the compiler.
