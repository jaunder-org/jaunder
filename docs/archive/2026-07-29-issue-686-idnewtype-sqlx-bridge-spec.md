# Spec — #686: sqlx bridges for `IdNewtype` / `NumNewtype` + retiring the `i64` residue

- Issue: [#686](https://github.com/jaunder-org/jaunder/issues/686)
- Milestone: Domain-value type safety (newtypes)
- Governing ADR: [ADR-0063](../adr/0063-domain-value-newtype-convention.md) §2,
  [ADR-0071](../adr/0071-sqlx-string-newtype-bridge.md) (amended by this issue)
- Blocks: [#697](https://github.com/jaunder-org/jaunder/issues/697) (adoption
  gate)
- Date: 2026-07-29

## Problem

**`StrNewtype` is the only newtype family with a sqlx bridge.** Verified by
`rg -c 'sqlx'` over `macros/src`: `str_newtype.rs` 66, `id_newtype.rs` **0**,
`num_newtype.rs` **0**.

`id_newtype.rs` emits `From<i64>`, `From<Self> for i64`, `Display`, `FromStr`,
and a transparent-i64 serde bridge — no `sqlx::Type`/`Encode`/`Decode`.
`num_newtype.rs` likewise. So neither an ID nor a bounded numeric can bind to or
decode from a column, and every SQL boundary falls back to the primitive.

Measured on `da17c14a`:

- **114** `.bind(i64::from(…))` sites across `storage/src` — every one a pure
  conversion tax, all of that exact shape.
- **9** bare-primitive declarations that exist only because the bridges do not,
  each followed by a hand re-wrap (`UserId::from(user_id)` at `helpers.rs:61` is
  the pattern).

Adoption is otherwise complete: `storage/src` has **161** id-named declarations
already typed. `PostRow`'s doc comment enumerates its deliberate primitives
(`rendered_html` per #502, `tags` as a JSON aggregate) without mentioning the
IDs, because the IDs are not a decision — they are a missing capability.

## The residue — every site

Named-field structs:

| Site                         | Now                                  | Should be |
| ---------------------------- | ------------------------------------ | --------- |
| `storage/src/helpers.rs:79`  | `build_session_record(user_id: i64)` | `UserId`  |
| `storage/src/helpers.rs:255` | `PostRow.post_id: i64`               | `PostId`  |
| `storage/src/helpers.rs:256` | `PostRow.user_id: i64`               | `UserId`  |

Tuple row aliases — positional, so no field name to grep; these were missed by
the audit's field-name scan and found only by enumerating `type X = ( … );` in
`storage/src`:

| Site                                          | Position | Now           | Should be                 |
| --------------------------------------------- | -------- | ------------- | ------------------------- |
| `storage/src/helpers.rs:31` `UserRecordParts` | 0        | `i64`         | `UserId`                  |
| `storage/src/helpers.rs:187` `UserRow`        | 0        | `i64`         | `UserId`                  |
| `storage/src/helpers.rs:203` `SessionRow`     | 1        | `i64`         | `UserId`                  |
| `storage/src/helpers.rs:229` `InviteRow`      | 4        | `Option<i64>` | `Option<UserId>`          |
| `storage/src/helpers.rs:275` `MediaRow`       | 0        | `i64`         | `UserId`                  |
| `storage/src/helpers.rs:275` `MediaRow`       | 5        | `i64`         | `ByteSize` _(NumNewtype)_ |

`MediaRecord.user_id: UserId` and `.size_bytes: ByteSize`
(`storage/src/media.rs:17,27`) confirm both are hand-converted today.

Verified rejects in the same aliases — do **not** sweep:

- `SessionRow:3` `String` (session label) — documented at `helpers.rs:214-218`:
  decoded as a plain `String` and repaired via `SessionLabel::from_lossy`,
  deliberately _not_ a validating decode, so a legacy out-of-range row does not
  fail the whole `list_sessions` query.
- `MediaRow:6` `Option<String>` (`source_url`) — `AbsoluteUrl` territory, owned
  by #675.
- `UserRecordParts:7,8` / `UserRow:7,8` — plain `bool` flags.

Inline `query_as::<_, ( … )>` tuples — audited 2026-07-29 (plan task 1). 31
scanned, 23 with bare positions, of which **18 sites / 20 positions** carry an
ID or bounded numeric. Each is followed by a hand re-wrap
(`SubscriptionId::from(id)` at `subscriptions.rs:182` is the pattern), which the
bridge deletes:

| Site                   | Column(s)                              | Should be                          |
| ---------------------- | -------------------------------------- | ---------------------------------- |
| `audiences.rs:178`     | `RETURNING audience_id`                | `AudienceId`                       |
| `audiences.rs:207`     | `RETURNING audience_id`                | `AudienceId`                       |
| `audiences.rs:257`     | `SELECT audience_id, …`                | `AudienceId`                       |
| `audiences.rs:331`     | `SELECT subscription_id`               | `SubscriptionId`                   |
| `email.rs:176`         | `RETURNING user_id, email`             | `UserId`                           |
| `password.rs:124`      | `RETURNING user_id`                    | `UserId`                           |
| `postgres/mod.rs:169`  | `RETURNING user_id`                    | `UserId`                           |
| `sqlite/mod.rs:295`    | `RETURNING user_id`                    | `UserId`                           |
| `postgres/posts.rs:40` | `SELECT user_id, deleted_at`           | `UserId`                           |
| `sqlite/posts.rs:42`   | `SELECT user_id, deleted_at`           | `UserId`                           |
| `posts.rs:1323`        | `SELECT pt.post_id, pt.tag_id, …`      | `PostId` **+** `TagId`             |
| `posts.rs:1556`        | `SELECT tag_id, tag_slug`              | `TagId`                            |
| `posts.rs:1568`        | `SELECT tag_id, tag_slug`              | `TagId`                            |
| `subscriptions.rs:176` | `SELECT_SUBSCRIPTION_ID`               | `SubscriptionId`                   |
| `subscriptions.rs:226` | `LIST_ACTIVE_SUBSCRIBERS`              | `SubscriptionId` **+** `ChannelId` |
| `subscriptions.rs:247` | `SELECT_LOCAL_CHANNEL_ID`              | `ChannelId`                        |
| `postgres/media.rs:14` | `COALESCE(SUM(size_bytes), 0)::bigint` | `ByteSize` _(NumNewtype)_          |
| `sqlite/media.rs:14`   | `COALESCE(SUM(size_bytes), 0)`         | `ByteSize` _(NumNewtype)_          |

`posts.rs:1323` is also a live transposition hazard — two adjacent bare `i64`s
that are `post_id` and `tag_id`.

Rejects among the inline tuples:

- `subscriptions.rs:212` `(i64,)` — an existence flag consumed as `exists != 0`,
  not an id.
- `subscriptions.rs:226` position 2 `String` — `subscriber_ref`, deliberately
  _polymorphic_ (a stringified user id in one arm, an external reference in
  another; ADR-0020, and ADR-0063 §1 models it as an enum, never a string
  newtype).
- `site_config.rs:365`, `:404`, `:417` and `user_config.rs:101` — config
  keys/values, owned by #687.

**Revised total:** 9 named/alias positions + 20 inline-tuple positions ≈ **29**
declaration positions, plus the accompanying hand re-wraps. Task 5 of the plan
is correspondingly larger than first scoped.

## Decision

### 1. Bridges for both derives — unconditional

Add a sqlx bridge to `#[derive(IdNewtype)]` **and** `#[derive(NumNewtype)]`,
modelled on `str_newtype.rs`'s `sqlx_impls_inner` (`:331`): generic
`Type`/`Encode`/`Decode` over `DB: sqlx::Database`, bounded on the inner type,
inside `#[cfg(feature = "sqlx")] const _: () = { … }`.

Simpler than the string bridge in one respect: `Decode` is a plain infallible
wrap (`Ok(Self(v))`) for `IdNewtype` — an ID has no value invariant, only the
transposition guarantee (ADR-0063 §2). `NumNewtype` is the exception and must
**re-run its bound** on decode, matching its serde bridge, which already rejects
out-of-range values on the wire; a `Decode` that skipped the bound would make
the column a hole in the invariant.

`NumNewtype`'s inner type is declared (`inner = u32|i64|usize|…`), so its bridge
is parameterized on that type rather than hardcoded to `i64`.

**No opt-out attribute** on either. `StrNewtype`'s `no_sqlx`/`sqlx` controls
exist for real cases (`RawToken` must never be stored; `InviteCode` is a stored
secret). No ID or numeric value has such a case; a flag is added when one
appears, not speculatively.

`macros/Cargo.toml:20` already declares the featureless `sqlx` feature that
resolves against the consuming crate — no dependency or manifest change.

### 2. Retire the residue

Type the 9 sites above; convert the **114** `.bind(i64::from(x))` → `.bind(x)`;
delete the now-dead hand re-wraps (`UserId::from(user_id)` and friends). Update
`PostRow`'s and the row aliases' doc comments so the remaining primitives are
exactly the documented rejects.

The 114-site bind sweep is delegated to a subagent so the file bulk stays out of
the driving context; the brief restates the house rules (no `ctx_*` MCP calls,
worktree absolute paths, `cargo xtask` never bare `nextest`).

### 3. Pin the regression

Extend `xtask/src/steps/sqlx_newtype_bind_check.rs` — today it flags `.as_ref()`
/ `&*` strips inside `.bind(` for string newtypes — to also flag `i64::from(`
inside a `.bind(`. Same file, same scan, same allowlist mechanism.

It must **provably bite**: a unit test asserting `.bind(i64::from(user_id))` is
flagged, alongside the existing `as_ref_strip_is_flagged` /
`deref_binds_are_flagged` cases.

The broader field/parameter adoption gate stays with #697, which this issue
unblocks.

### 4. `ResolutionBinds` — delete the sentinels, bind NULL

`storage/src/posts.rs:1698` currently fakes "no such viewer" with out-of-domain
values:

```rust
struct ResolutionBinds {
    author_id: i64,  // sentinel -1 for Anonymous
    channel: i64,    // sentinel -1 for Anonymous
    subref: String,  // sentinel "" for Anonymous
}
```

**Decision (owner, 2026-07-29): remove the sentinels entirely.** The struct
becomes `Option<UserId>` / `Option<ChannelId>` / `Option<String>`, and `None`
binds as SQL NULL — no `-1`, no `""`, anywhere.

This is sound because SQL's three-valued logic already does what the sentinels
simulate, and `resolution_where`'s fragment (`:1748-1767`) contains **no `NOT`**
— the one construct that would break the equivalence (`NOT FALSE` includes a
row; `NOT NULL` excludes it):

- `p.user_id = $author` with NULL yields NULL. In the outer `NULL OR EXISTS(…)`:
  `NULL OR TRUE` = TRUE, `NULL OR FALSE` = NULL → excluded by `WHERE`. Identical
  to the `FALSE OR …` outcomes today.
- In both `EXISTS` subqueries a NULL bind makes the `AND` chain NULL, so no row
  qualifies and **`EXISTS` yields FALSE, never NULL** — no NULL escapes to
  contaminate the outer `OR`.

It also retires a _second_ sentinel at `:1739`,
`subscriber_ref.parse::<i64>().unwrap_or(-1)` ("this channel viewer has no local
user id"), which becomes `subscriber_ref.parse::<UserId>().ok()` — `IdNewtype`
already generates the `FromStr` this needs.

**Correctness, not just cleanliness.** `subscriber_ref` is `TEXT NOT NULL` on
both backends
(`storage/migrations/{sqlite,postgres}/0019_create_subscriptions.sql:5`) with
**no non-empty CHECK**, so a row with `subscriber_ref = ''` is schema-legal —
and an anonymous viewer, which binds `""` today, would match it and be shown
subscriber-targeted posts. Whether such a row is reachable through the current
insert path is unverified (plan task), but NULL removes the class of bug rather
than resting on the assumption. The same argument applies more weakly to
`author_id = -1`, whose safety rests on the commented claim that "no post has
`user_id` -1".

Consequences:

- The doc comments at `:1714-1717` and `:1699-1707` document the sentinel scheme
  and must be rewritten.
- `bind_onto` binds these five placeholders; its signature takes `Option`
  values, which needs the `IdNewtype` bridge from §1 — so this sequences
  **after** §1.
- Needs dual-backend tests: anonymous sees exactly the public posts (unchanged
  behaviour), plus a regression test seeding a `subscriber_ref = ''`
  subscription and asserting an anonymous viewer cannot see its posts.

## Non-goals

- The broader newtype-adoption gate (#697).
- `PostRow.rendered_html` (#502) and `PostRow.tags` stay `String`;
  `MediaRow:6 source_url` belongs to #675.
- No wire-shape change: the serde bridges already render an ID as a bare integer
  and a numeric as its inner integer; the sqlx bridge does not touch them.

## Verification

- `cargo xtask check`, then `cargo xtask validate --no-e2e` before push; full
  `validate` deferred to CI per ADR-0034.
- `macros` **is** coverage-measured — cover the new bridges' paths;
  derive-expansion tests use `syn::parse_quote!`.
- Both backends must be exercised: the generic bridges are only correct if the
  inner type resolves for **both** `Sqlite` and `Postgres`. Dual-backend storage
  tests cover this; bind whole `TestEnv` per ADR-0053.
- `NumNewtype`'s decode-side bound needs a test proving an out-of-range
  **column** value is rejected, not just an out-of-range wire value.
- The bind gate's new case must fail on a deliberately reverted hunk.

## Risks

- **Inference regressions at bind sites.** Removing `i64::from(…)` can leave
  sqlx unable to infer a placeholder's type in a dynamically-built query. Expect
  a handful of the 114 to need an explicit turbofish; the subagent brief must
  say these are _not_ to be reverted to `i64::from`.
- **`NumNewtype` decode strictness is a behaviour change.** If any existing row
  holds an out-of-range value, a bound-checking `Decode` turns a
  silently-accepted read into a decode error. `ByteSize` (`min` only, no max) is
  low risk, but this must be confirmed per adopting type before the bridge is
  applied to it — and it is the reason `SessionLabel`-style lossy repair exists
  elsewhere.
- **`FromRow` coupling.** `PostRow` decodes by column name; the bridge must
  satisfy `#[derive(sqlx::FromRow)]` without a `#[sqlx(try_from)]` attribute, as
  string newtypes do.
- Wide mechanical diff over `storage/src`; #692 and #697 are the neighbours,
  neither started.
