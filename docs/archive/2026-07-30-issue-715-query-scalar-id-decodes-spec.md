# Issue #715 — id decodes: type the residue, and build a gate that enumerates

**Issue:** [#715](https://github.com/jaunder-org/jaunder/issues/715) **Status:**
approved **Date:** 2026-07-30

## Problem

#686 gave `IdNewtype`/`NumNewtype` an sqlx bridge (ADR-0071) and swept the
primitive residue at the row/bind boundary. Its audit enumerated
`query_as::<_, ( … )>` — a **syntactic proxy** for "row decode site". Every
decode spelled another way survived: `query_scalar`, `query_as` with the type on
the `let` instead of a turbofish, and `row.get`. Those survivors decode ids as
bare `i64` and hand-rewrap them (`PostId::from(…)`).

The `sqlx-newtype-bind` gate does not catch them: it polices **binds**, not
decodes.

### The deeper problem — the Jurassic Park problem

Each audit pass found precisely what its chosen spelling could match, and
reported done. #686's own spec records that its first field-name audit missed
five sites because tuple positions have no names; the tuple audit then missed
these because they are not tuples. A check that searches for the shape you
already expect can only ever confirm your hypothesis — it can never discover
that reality is bigger than the hypothesis.

That failure mode governs the gate design below, and is the reason this issue
ships a gate at all rather than only a sweep.

### Correction: what typing an id actually buys

**The issue's stated justification is wrong, and this spec does not inherit
it.** #715 claims that typing `tag_id` as `TagId` makes transposing the
`post_tags` binds "a compile error, which is the entire point of ADR-0063 §2".
It does not. sqlx's signature is
`pub fn bind<T: 'q + Encode<'q, DB> + Type<DB>>(self, value: T) -> Self`
(`sqlx-core/src/query.rs:86`) — every `.bind` call is independently generic, so
`PostId` and `TagId` both satisfy the bound and `.bind(tag_id).bind(post_id)`
compiles exactly as well as the correct order. The same is true of `FromRow`
tuple positions, so #686's `query_as::<_, (PostId, TagId, Tag, TagLabel)>` at
`posts.rs:1339` does not close a transposition at its decode site either.

What typing actually buys is **downstream**: once `tag_id` is a `TagId`, it
cannot be passed to a `PostId` parameter, assigned to a `PostId` field, or
compared against a `PostId`. That is where ADR-0063 §2's protection lives and it
is real — it is simply not at the `.bind` chain. The value of this work is that
ids reach the domain already typed, so every subsequent use is checked; it is
not that the SQL statement's argument order becomes compiler-verified.

Recorded here rather than silently dropped, because the false claim is
load-bearing in the issue text and will otherwise be repeated by the next
reader.

## Decisions

### D1 — Audit by column role, not call shape

The site list is derived by asking "does this decode yield an id?", independent
of how the decode is spelled. That produces **11 sites in `storage/src`**, where
the issue's own table (built by grepping `query_scalar`) lists 8.

### D2 — `FeedEventId` already exists

The issue asks for a decision on whether the feed-event id "earns a newtype". It
has one: `FeedEventId` is an `IdNewtype` at `common/src/ids.rs:44`, and
`FeedEventStorage::enqueue` already returns `FeedEventId`. There is no decision
to make — only a hand-rewrap to delete. The issue's claim is stale.

### D3 — The gate enumerates; it does not search

A new syn-AST gate, `sqlx-newtype-decode`, sibling to `sqlx-newtype-bind`.

> In `storage/src`, every sqlx decode target that resolves to `i64` or
> `Option<i64>` — including through `Vec<…>`, `Result<…>`, and tuple positions —
> is a **failure** unless that exact decode site appears in an enumerated
> allowlist carrying a written reason.

**Nothing self-exempts.** The rule performs no inspection of the SQL: it does
not look for `*_id` to decide something is an id, and it does not look for
`COUNT(` to decide something is a count. Both are pattern searches, and either
one hands the blind spot straight back —
`SELECT post_id FROM t WHERE (SELECT COUNT(*) …) > 0` defeats the second while
looking perfectly safe.

The consequence, which is the point: a decode the gate has never seen before is
a failure _because it recognised nothing_, not because it recognised a
violation.

**Line-based is not an option.** The decode type and the SQL literal sit on
different lines at nearly every site, and the ascription form puts the type on a
`let` several lines above the query. syn relates them as one expression; a line
scan cannot.

### D4 — The population is "decode targets whose type is written down"

syn has no type inference, so the population must be defined by _where the type
is declared_. Four readable positions, all of which occur live:

| Position                                                                  | Live example                                                                                                                  |
| ------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------- |
| Turbofish on the call                                                     | `query_scalar::<_, i64>(…)` — `users.rs:267`                                                                                  |
| Ascription on the enclosing `let`                                         | `let id: i64 = query_scalar(…)` — `feed_events.rs:178`; `let rows: Vec<(String, Option<i64>)> = query_as(…)` — `posts.rs:952` |
| Return type of the enclosing `fn`, when the decode is the tail expression | `scalar_i64(…) -> Result<i64, sqlx::Error>` — `test_support.rs:109-112`                                                       |
| Declaration of a named `query_as` target                                  | `PostRow`, `UserRow`, `CacheTuple` in `helpers.rs` / `feed_cache.rs`                                                          |

Matching **recurses** through `Vec`, `Option`, `Result`, and tuples, so
`Vec<(String, Option<i64>)>` is in population. Site #9 is exactly this shape and
would otherwise be missed by the gate written to catch it.

The fourth row closes the hole that would otherwise sit under D6: a future
`struct PostRow { revision_id: i64 }` is a decode of an id into a primitive, and
it is readable, so it is policed. Every named target type today is fully
newtyped, so this costs no churn now.

**The first three positions overlap, and the precedence rule is load-bearing.**
Live code hits two at once: `postgres/backup.rs:320-326` is a
`-> Result<i64, BackupError>` function whose body is
`query_scalar::<_, Option<i64>>(…)…?`, so the turbofish says `Option<i64>` and
the return type says `i64`. Without a rule the gate records both, the
allowlist's declared count never matches, and it fails on a clean tree. The
rule:

> **One record per decode call expression.** Its target is the nearest declared
> type — turbofish first, then the enclosing `let` ascription, then the
> enclosing `fn` return.

"Per call" also means a `let` or `fn` covering several calls yields one record
each: `backup.rs:733-745` is one `let live_count: i64 = match { … }` over two
`query_scalar` calls and must produce two records — entries A1/A2 below. The
same property is what `test_support.rs`'s `scalar_i64` entry (`count: 2`) rests
on. Position 4 is a separate population, recorded per declared field, not per
call.

### D5 — What the gate cannot read, stated rather than papered over

Two live constructs are **outside** the population because syn cannot resolve
them, and the honest posture is to say so rather than to reach for a heuristic:

- **A `.get`/`try_get` with neither turbofish nor ascription.** syn cannot tell
  `sqlx::Row::get` from `serde_json::Map::get`, and both occur in the policed
  root: `postgres/backup.rs:177` and `sqlite/backup.rs:160` are
  `let value = row.get(…)` on a JSON map. Keying on the receiver name (`row` vs
  `r`) to separate them would be a pattern search — forbidden by D3 — and would
  also miss the two real sites, whose receiver is `r`.
- **A decode whose type is pinned only by later use** (e.g. an unascribed `let`
  whose value is later pushed into a `Vec<i64>`).

Neither occurs as an id decode today. Both are recorded in the gate's module doc
as known-unreadable, so the next audit inherits the boundary instead of
rediscovering it. This is the ADR's own posture: the gate defines a population
it can read and says nothing about what it cannot, rather than implying by a
green run that it looked everywhere.

### D6 — `row.get` in struct-literal position is covered by its destination type

`id: r.get("id")` inside a `FeedEventRecord { … }` literal has its type pinned
by the field's declared type. That declaration is itself in the population via
D4's fourth row, so the invariant is enforced where the newtype belongs — on the
struct — rather than at each construction site.

### D7 — An allowlist entry names one decode and its multiplicity

Load-bearing, and **not** the substring-needle mechanism `sqlx-newtype-bind`
uses. That gate's own doc concedes "a needle exempts every matching line under
`POLICED_ROOT`, not one site" — a region-scoped exemption, which re-creates the
blind spot one level down. The live population contains byte-identical decode
sites that no needle could separate: `backup.rs:773`/`:778`,
`test_support.rs:111`/`:112`, and `postgres/posts.rs:148`/`sqlite/posts.rs:164`.

An entry is therefore keyed by **(file, enclosing function path, the decode's
rendered target type and SQL/column text, expected occurrence count)** — all
reflow-stable, none positional. Identical siblings are covered by one entry
stating `count: 2`. A third identical decode appearing later makes the observed
count 3, the entry's count no longer matches, and the gate **fails**. A
different decode in the same function matches no entry and fails. Neither can be
absorbed silently, which is what "one decode, never a region" has to mean to be
worth anything.

### D8 — ADR: "enumerate, don't search", stated generally

A new ADR records the principle for **all** static type-safety gates, not just
decode gates.

Stating it generally makes `sqlx-newtype-bind` **known non-conforming** on the
day the ADR lands, on two counts: it decides violations by searching for three
strip spellings (`.as_ref()`, `&*`, `i64::from(` —
`sqlx_newtype_bind_check.rs:95-101`), and its `ALLOWLIST` is region-scoped by
its own admission (`:60-62`). #716 is open because a strip laundered through an
`i64` parameter has none of those spellings.

The ADR names both gaps and names #716 as the outstanding work. The bind gate is
**not** rebuilt in this cycle. Because the ADR asserts a scope for #716 that
#716's own body does not currently carry, this cycle updates that issue to
match.

### D9 — Scope boundaries held

- **`raw_ids` / `purge_corrupt` stay.** Typing the feed-events decode makes
  `corrupt: Vec<FeedEventId>` and `purge_corrupt(&[FeedEventId])` follow
  naturally. `raw_ids(&[FeedEventId]) -> Vec<i64>` at
  `postgres/feed_events.rs:40` is a strip laundered through a helper —
  bind-side, #716's class. Left alone.
- **`audience_target_row` comes in.** `posts.rs:1838-1846` returns
  `(&'static str, Option<i64>)` via `Some(i64::from(*id))` — the exact encode
  twin of the `audience_target_from_row` decode being fixed. Leaving one half
  typed and the other stripped would be a worse state than either. It is
  bind-side, so this is a deliberate one-function extension past the decode
  boundary, not a general widening.
- **The gate's root stays `storage/src`.** The two out-of-root sites in
  `server/tests/storage/mod.rs` are fixed by the sweep but not policed; a
  regression there surfaces as a failing test, not a production transposition.
  Recorded in the gate's module doc so the next audit knows the boundary is
  deliberate.
- **Test code inside `storage/src` is policed.** `#[cfg(test)]` modules and
  `test_support.rs` are in the scanned root and take allowlist entries like
  anything else.

## The population, enumerated

The ADR's thesis is that a population must be enumerated, not characterised.
Applying that to this spec: below is **every** member, both violations and
exemptions. An implementer who has to grep to fill a gap here has been handed
the failure mode the work exists to close.

### Violations — 11 sites, all in `storage/src`

| #   | Site                         | Position (D4)                      | Decodes                                | Becomes              |
| --- | ---------------------------- | ---------------------------------- | -------------------------------------- | -------------------- |
| 1   | `users.rs:267`               | turbofish                          | `INSERT … RETURNING user_id`           | `UserId`             |
| 2   | `postgres/mod.rs:120`        | turbofish                          | `INSERT … RETURNING user_id`           | `UserId`             |
| 3   | `sqlite/mod.rs:236`          | turbofish                          | `INSERT … RETURNING user_id`           | `UserId`             |
| 4   | `posts.rs:908`               | turbofish                          | `SELECT post_id FROM idempotency_keys` | `PostId`             |
| 5   | `posts.rs:1917`              | turbofish                          | `INSERT … RETURNING post_id`           | `PostId`             |
| 6   | `postgres/posts.rs:148`      | turbofish                          | `SELECT tag_id FROM tags`              | `TagId`              |
| 7   | `sqlite/posts.rs:164`        | turbofish                          | `SELECT tag_id FROM tags`              | `TagId`              |
| 8   | `feed_events.rs:178`         | **`let` ascription**               | `INSERT … RETURNING id`                | `FeedEventId`        |
| 9   | `posts.rs:952`               | **`let` ascription, `Vec<tuple>`** | `SELECT tk.name, pa.audience_id`       | `Option<AudienceId>` |
| 10  | `postgres/feed_events.rs:77` | **`let` ascription, `row.get`**    | `RETURNING id, …`                      | `FeedEventId`        |
| 11  | `sqlite/feed_events.rs:77`   | **`let` ascription, `row.get`**    | `RETURNING id, …`                      | `FeedEventId`        |

Sites 9–11 are absent from the issue's table. #9 is a genuine **#686 miss** —
its audit required a turbofish and this site writes the type on the `let`.

### Exemptions — 10 allowlist entries covering 12 sites

**Corrected during implementation: 10 entries, not 9.** Entry A was written as
one `count: 2` entry, but its two dialect arms issue _different_ SQL
(`sqlite_master` vs `information_schema.tables`), and the SQL text is part of
the entry key (D7). Two distinct keys cannot share one entry, so A is two
`count: 1` entries. Forced by the site-scoping rule rather than chosen — and the
rule working as intended: a single entry spanning both arms would have been a
small region exemption.

| Entry | Site(s)                       | Count | Reason                                                                           |
| ----- | ----------------------------- | ----- | -------------------------------------------------------------------------------- |
| A1    | `backup.rs:734`               | 1     | `COUNT(*)` of live SQLite tables, checked against the backup manifest            |
| A2    | `backup.rs:739`               | 1     | `COUNT(*)` of live Postgres tables, the dialect twin of A1                       |
| B     | `backup.rs:773`, `:778`       | 2     | `COUNT(*)` per seeded table; byte-identical, SQL built in `format!`              |
| C     | `sqlite/mod.rs:150`           | 1     | `SELECT EXISTS(…)` decoded as `i64` (SQLite has no bool); SQL built in `format!` |
| D     | `postgres/schema.rs:22`       | 1     | `COUNT(*)` of non-deferrable FK constraints                                      |
| E     | `postgres/backup.rs:322`      | 1     | `MAX(version)` migration version, `Option<i64>`                                  |
| F     | `sqlite/backup.rs:291`        | 1     | `MAX(version)` migration version, `Option<i64>`                                  |
| G     | `test_support.rs:111`, `:112` | 2     | Generic test scalar helper; SQL is a runtime `&str`, type from the `fn` return   |
| H     | `subscriptions.rs:215`        | 1     | `(i64,)` existence flag, not an id (`subscriptions.rs:149` already says so)      |
| I     | `sqlite/feed_events.rs:86`    | 1     | `attempts` retry counter                                                         |

### Out of population — verified, no entry needed

Decode target is not `i64`-family: `posts.rs:1378`, `posts.rs:1470`,
`postgres/posts.rs:132`, `sqlite/posts.rs:149`, `postgres/teardown.rs:34`,
`test_support.rs:533` (all `bool`); `postgres/mod.rs:296`, `sqlite/mod.rs:140`
(`String`); `postgres/mod.rs:307` (`bool`); `postgres/feed_events.rs:90`
(`attempts` into an `i32` struct field).

Unreadable per D5: `postgres/backup.rs:177`, `sqlite/backup.rs:160` (JSON map
`.get`).

### Ripples

- Generic `FromRow` bounds move with their sites: `(i64,)` at `posts.rs:825` →
  `(PostId,)`, `posts.rs:1911` → `(PostId,)`, `feed_events.rs:170` →
  `(FeedEventId,)`; `(String, Option<i64>)` at `posts.rs:829` →
  `(String, Option<AudienceId>)`. `subscriptions.rs:150`'s `(i64,)` is entry H
  and stays.
- `Encode`/`Decode` bounds move too, and they are easy to miss because an unused
  `where` bound does not warn: `for<'r> i64: sqlx::Decode<'r, DB>` at
  `users.rs:227` gains `UserId` (that impl has no `FromRow` tuple bound at all),
  and `for<'q> Option<i64>: sqlx::Encode<'q, DB>` at `posts.rs:851`, `:1893`,
  and `:1979` becomes `Option<AudienceId>` once `audience_target_row` is typed.
- **The compiler is the authority on the bound set, not this list.** After the
  sweep, every `i64`-family bound in a touched `impl` is tried for removal and
  the removal kept if the crate still builds — a bound left stale by this change
  is invisible to both rustc and clippy.
- `audience_target_from_row(kind, Option<i64>)` → `Option<AudienceId>`;
  `audience_target_row` → `(&'static str, Option<AudienceId>)` (D9); both
  `AudienceId::from` and `i64::from` in the pair disappear.
- `corrupt: Vec<i64>` / `purge_corrupt` in both feed-events dialects become
  `FeedEventId`-typed. The Postgres `purge_corrupt` binds its slice to
  `DELETE … WHERE id = ANY($1)`, and the ADR-0071 bridge emits no
  `PgHasArrayType`, so it routes through the existing `raw_ids` helper
  internally — the same shape `mark_regenerated`/`mark_pinged`/`mark_failed`
  already use.

### Outside the gate's root

- `server/tests/storage/mod.rs:240-253` — `local_channel_id`,
  `let raw: i64 = query_scalar(…)` then `ChannelId::from(raw)` → `ChannelId`.
- `server/tests/storage/mod.rs:2446-2470` — `post_audience_rows`, the
  test-harness twin of site #9, returns `Vec<(String, Option<i64>)>` →
  `Option<AudienceId>`; its `.bind(i64::from(post_id))` at `:2458`/`:2465`
  becomes `.bind(post_id)`.

### Incidental

`storage/src/email.rs:106`'s comment cites a `(i64, Email): FromRow` bound that
#686 changed to `(UserId, Email)` at line 101. Same change lineage; corrected
here.

## Acceptance criteria

**Sweep**

1. Each of the 11 sites in the violations table decodes into the type in its
   "Becomes" column, and the hand re-wrap that followed it is **deleted**, not
   moved. Verified by diffing each named file:line.
2. These eight rewraps — the complete set of id-newtype constructions in
   `storage/src` whose argument comes from a decode — no longer exist:
   `posts.rs:1860`, `posts.rs:1943`, `users.rs:285`, `postgres/mod.rs:134`,
   `sqlite/mod.rs:250`, `feed_events.rs:183`, `postgres/feed_events.rs:87`,
   `sqlite/feed_events.rs:88`. Enumerated rather than expressed as a grep,
   because "came from a decode" is a dataflow judgment no grep can make:
   `post_service.rs:525`'s `ChannelId::from(0)` is production code constructing
   an id from a literal placeholder and is **not** in scope.
3. `posts.rs:1843`'s `Some(i64::from(*id))` is gone and `audience_target_row`
   returns `Option<AudienceId>` (D9).
4. `server/tests/storage/mod.rs`'s `local_channel_id` returns `ChannelId` with
   no `ChannelId::from` rewrap, and `post_audience_rows` returns
   `Vec<(String, Option<AudienceId>)>` with no `i64::from(post_id)` bind.
5. Every generic `FromRow` bound listed under Ripples carries its new type; the
   crate builds for both backends.
6. The PR body states what typing these ids does and does not buy, per the
   Correction section — specifically that it does **not** make a `.bind`
   transposition a compile error. No commit-local "transposition is now a
   compile error" experiment is attempted; it would fail.

**Gate**

7. `cargo xtask` runs a step named `sqlx-newtype-decode` that parses
   `storage/src` with syn and reports pass/fail per D3.
8. The gate **bites** on all four D4 positions: unit tests assert a flag for
   `query_scalar::<_, i64>`, `let x: i64 = query_scalar(…)`,
   `let x: Vec<(String, Option<i64>)> = query_as(…)`, `let x: i64 = row.get(…)`,
   a decode whose type comes from the enclosing `fn` return, and a named target
   type with an `i64` field.
9. The gate **does not over-bite**: unit tests assert
   `query_scalar::<_, PostId>`, a `bool`/`String` decode target, and a
   struct-literal `row.get` (D6) are all clean.
10. A unit test asserts an allowlist entry's **count** is load-bearing: an entry
    with `count: 2` passes on two matching decodes and **fails** on three.
11. A unit test asserts an entry exempts only the decode it names — a different
    `i64` decode in the same function still fails (D7).
12. Reverting any one of the 11 sweep sites to its `i64` decode makes the gate
    fail. Demonstrated for at least one site and stated in the PR body.
13. The allowlist contains exactly the 10 entries in the exemptions table, each
    with a prose reason, and **no** entry for a decode that yields an id.
14. A missing/renamed `storage/src` is a hard gate failure, not a silent skip —
    matching `sqlx-newtype-bind`'s `POLICED_ROOT` behaviour.
15. The gate's module doc records the two unreadable classes from D5 and the
    `storage/src`-only root from D9, each with its reason.

**ADR**

16. A numberless draft in `docs/adr/drafts/` states "enumerate, don't search"
    for all static type-safety gates, records `sqlx-newtype-decode` as
    conforming, and records `sqlx-newtype-bind` as non-conforming on **both**
    the spelling-search and the region-scoped-allowlist counts.
17. The ADR is internally consistent about #716: an enumerating bind gate
    **would** flag the laundered bind (a bare `i64` bound in
    `list_published_in_window_rows`), while still being unable to attribute it
    to a newtype strip in the caller. Detection and attribution are stated
    separately.
18. `sqlx_newtype_bind_check.rs`'s module doc links the ADR at its existing
    "what it still cannot see" note.
19. Issue #716 is updated so its scope matches what the ADR asserts for it,
    rather than the ADR unilaterally re-scoping an issue that still offers two
    options.

**Whole**

20. `cargo xtask check` is green, including coverage and the new gate.
