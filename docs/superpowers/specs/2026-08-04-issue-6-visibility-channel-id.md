# Issue #6 — the author branch must fire only for a local viewer

**Issue:** [#6](https://github.com/jaunder-org/jaunder/issues/6) — _visibility:
author-branch resolution ignores channel_id (privacy hole once remote channels
exist)_. Milestone: Correctness & data integrity. Type: Bug.

## The defect

`resolution_where` (`storage/src/posts.rs:1974-2023`) builds the visibility
filter applied to every viewer-taking read. Its author branch is:

```sql
( p.user_id = ${author}
  OR EXISTS ( ... audience branches ... ) )
```

and `author` is bound from

```rust
let author_id = subscriber_ref.parse::<UserId>().ok();
```

— nothing else. The two audience branches correctly constrain
`s.channel_id = ${…}`; the author branch has **no channel predicate at all**.

`ViewerIdentity::Channel { channel_id, subscriber_ref }`
(`common/src/visibility.rs:76-83`) carries `channel_id` as an opaque FK into
`channels`. Nothing in the type records that a viewer is _local_, so ADR-0020's
rule — the author branch fires only for `Channel{local, author}` — is not
expressible without an ambient lookup, and the shipped code does not attempt it.
The in-code comment at `storage/src/posts.rs:1981-1984` claims the branch "fires
only for a local viewer"; that claim is aspirational and unenforced.

**Impact.** Inert today: only `Anonymous` and `ViewerIdentity::local(...)` are
ever constructed. The moment Layer B constructs a remote viewer — HTTP-Signature
Authorized Fetch yielding `Channel{activitypub, <actor URI>}` per
`docs/superpowers/specs/2026-06-19-content-visibility-layer-c-design.md:269-270`
— any remote `subscriber_ref` that happens to be the decimal string of a local
user id (`"42"`) matches that user's author branch and reads **all** of their
private, subscribers-only and named-audience posts.

**Second instance of the same hole.** `viewer_user_id`
(`common/src/visibility.rs:118-123`) carries the identical
`subscriber_ref.parse::<UserId>().ok()` assumption and decides `is_author`
(`web/src/posts/server.rs:37`), which gates owner-only controls. Fixing only the
SQL would close the read path and leave the authoring UI open to the same
confusion. It is in scope here.

**Third strand.** `storage/src/post_service.rs:526` deliberately _exploits_ the
defect: it re-reads a just-created post as its author using
`ViewerIdentity::local(user_id, ChannelId::from(0))`, with a comment stating the
channel id is irrelevant. `0` is not a real `channels` row.

## Decisions

1. **Locality becomes a type-level fact, not a runtime comparison.**
   `ViewerIdentity` splits its `Channel` variant:

   ```rust
   pub enum ViewerIdentity {
       Anonymous,
       Local { user_id: UserId },
       Remote { channel_id: ChannelId, subscriber_ref: String },
   }
   ```

   The author branch fires on `Local` and on nothing else. There is no
   constructible value that is remote _and_ matches an author — the illegal
   state is removed rather than guarded.

2. **`Local` carries no `ChannelId`.** A local viewer's channel is not a free
   parameter; it is always the `local` row. Queries resolve it inline in SQL
   (`channels.name` is `NOT NULL UNIQUE` on both backends,
   `storage/migrations/{sqlite,postgres}/0018_create_visibility_lookups.sql:3`,
   so the subquery is uncorrelated and yields at most one row):

   ```sql
   AND s.channel_id = (SELECT channel_id FROM channels WHERE name = 'local')
   ```

   This deletes the `ChannelId::from(0)` placeholder outright and takes the
   process-global `LOCAL_CHANNEL_ID` cache off the visibility path entirely — a
   stale or cross-pool-leaked cached id can no longer misresolve content.

3. **The channel predicate is emitted per variant, with variable bind arity.**
   In `resolution_where`, `Local` binds 3 placeholders (author, ref, ref);
   `Remote` and `Anonymous` bind 5 as today. `resolution_where` already returns
   the next free placeholder index, and this is safe at all **14** call sites
   (`storage/src/posts.rs:1077, 1161, 1314, 1344, 1386, 1414, 1595, 1627, 1687, 1721, 2313, 2324, 2335, 2347`):
   the ten that have trailing binds thread `next` through their format strings,
   and the four that discard it place the fragment last, as does `window_sql`
   (`:2368`), which interpolates `{resolution}` as the final predicate before
   `ORDER BY`.

   Rejected: `COALESCE($chan, (SELECT …))` for a uniform template — it would
   coalesce `Anonymous` onto the local channel too, leaving anonymous safety
   resting indirectly on the NULL `subscriber_ref` bind, which is precisely the
   load-bearing subtlety #686 already had to remove once.

4. **`ResolutionBinds` becomes variant-shaped.** Today it is three independent
   `Option`s bound unconditionally in fives (`storage/src/posts.rs:1933-1944`,
   `:2046-2052`). Per-variant arity cannot be recovered from three `Option`s
   except by an implicit `(Some, None, Some) ⇒ Local` triple — reintroducing the
   very encoding decision 1 removes. It becomes an enum mirroring
   `ViewerIdentity`, and `bind_onto` matches on it:

   ```rust
   enum ResolutionBinds {
       Anonymous,
       Local { user_id: UserId, subref: String },
       Remote { channel: ChannelId, subref: String },
   }
   ```

5. **`is_subscriber` gets a second per-backend statement.** It does not build
   SQL; it executes a fixed 3-bind const, in two different placeholder dialects
   (`storage/src/postgres/subscriptions.rs:23-27` uses `$1/$2/$3`,
   `storage/src/sqlite/subscriptions.rs:20-24` uses `?/?/?`). Rather than change
   the existing const's arity for every caller, each backend gains an
   `IS_ACTIVE_LOCAL_SUBSCRIBER` const: the same query with `s.channel_id` bound
   to the decision-2 subquery and only 2 binds (author, ref).
   `IS_ACTIVE_SUBSCRIBER` is unchanged and serves the `Remote` arm.

6. **The orphaned memoized accessor is deleted.** With viewer construction no
   longer needing a channel id, `storage::local_channel_id` (the free fn over
   `static LOCAL_CHANNEL_ID: OnceLock<ChannelId>`,
   `storage/src/subscriptions.rs:84-102`) loses its only production caller
   (`web/src/viewer.rs:55`). Both are removed, along with its dual-backend test
   (`storage/src/subscriptions.rs:266-285`) and the trait method's doc reference
   to it (`:73-75`).

   The **trait** method `SubscriptionStorage::local_channel_id()` stays: it is
   still needed on the subscription **write** path — `subscribe`
   (`web/src/subscriptions/api.rs:27`) and `unsubscribe` (`:43`) insert and
   delete rows keyed by channel id. Its third caller, `is_subscribed` (`:65`),
   is a **read** path that exists only to build a viewer, and is deleted.

   Issue #342 (the OnceLock ignoring its subscriptions argument) is **not**
   closed from this cycle; the ship step comments on it noting its subject no
   longer exists.

7. **The AtomPub `SubscriptionStorage` DI seam is removed.**
   `owner_viewer(subscriptions, auth_user)`
   (`server/src/atompub/posts.rs:219-225`) does the channel lookup solely to
   construct a viewer; under decision 2 it collapses to an infallible
   `ViewerIdentity::Local { user_id }` and its `subscriptions` parameter dies.
   That parameter is the _only_ reason `Extension<Arc<dyn SubscriptionStorage>>`
   is in the axum router (`server/src/lib.rs:48-51`, `:126`; consumed at
   `server/src/atompub/posts.rs:43, 56, 220, 234, 261, 298`) — no other
   `server/src` handler takes it. The extension, its threading, and the
   now-false router comment are removed rather than left as dead plumbing. The
   separate Leptos context (`server/src/context.rs:33`) is untouched.

8. **ADR-0020 is amended in place.** Its viewer clause
   (`docs/adr/0020-content-visibility-and-subscription-model.md:78-80`) reads
   "never a bare local user id". Under decision 2 a local viewer _is_ a bare
   local user id in Rust — the `(channel, subscriber_ref)` pair is reconstructed
   in SQL rather than carried in the type. The ADR's resolution rule itself is
   unchanged and stays authoritative; the clause is amended with a dated note
   recording why the field disappeared. No new ADR.

   **ADR-0063** (`docs/adr/0063-domain-value-newtype-convention.md:77-81`) cites
   `ViewerIdentity::Channel`'s polymorphic `subscriber_ref` as its canonical
   "model it as an enum" example. The rule survives — this change _is_ the rule
   applied — but the named example ceases to exist, so the citation is updated
   to the new shape.

### Considered and declined

Collapsing the `Anonymous` arm to just the `public` EXISTS (its two subscription
branches can never match) is a real simplification but carries no correctness
content, rewrites SQL this issue does not concern, and widens the test surface.
Not filed as a follow-up — it is a micro-optimization, not a defect.

## Acceptance criteria

**AC1 — the type makes remote-as-author unconstructible.**
`common::visibility::ViewerIdentity` has exactly the three variants `Anonymous`,
`Local { user_id: UserId }`,
`Remote { channel_id: ChannelId, subscriber_ref: String }`.
`ViewerIdentity::local` takes one argument (`user_id: UserId`); `account_viewer`
is removed, and `web/src/viewer.rs` constructs `Local { user_id }` directly on
the authenticated path and `Anonymous` otherwise.

**AC2 — a remote viewer never matches the author branch.** `resolution_where`
binds the author placeholder to a value only for `ViewerIdentity::Local`. For
`Remote` and `Anonymous` the author bind is NULL.

**AC3 — the local channel is resolved in SQL.** For a `Local` viewer, both the
`subscribers` and `named` branches of `resolution_where` constrain
`s.channel_id = (SELECT channel_id FROM channels WHERE name = 'local')`, and no
Rust call site supplies a channel id in order to construct a viewer.

**AC4 — the adversarial case is denied, on both backends.** In the dual-backend
`resolution_matrix` (`server/tests/storage/mod.rs:5707`), with a second channel
row seeded (`INSERT INTO channels (name) VALUES ('activitypub')`), a viewer
`Remote { channel_id: <activitypub>, subscriber_ref: <author's user_id as a decimal string> }`
sees the author's **Public** post and does **not** see the author's Private,
Subscribers-only, Named(G) or Named(G2) posts — asserted through both
`get_post_by_id` and presence in `list_published`, under `#[apply(backends)]`
(SQLite and PostgreSQL).

**AC5 — the pre-existing truth table is unchanged.** Every existing
`resolution_matrix` cell (author / subscriber / named member / non-subscriber /
anonymous, across the six seeded posts) still holds, on both backends.

**AC6 — `is_author` is not spoofable.** `viewer_user_id` returns `Some(id)` only
for `ViewerIdentity::Local`, and `None` for `Remote` regardless of
`subscriber_ref`. A unit test in `common/src/visibility.rs` asserts
`viewer_user_id(&Remote { channel_id: _, subscriber_ref: "42".into() })` is
`None`. The four existing unit tests at `common/src/visibility.rs:379-419` are
updated or removed to match the new shape (`account_viewer_*` lose their
subject), with no net loss of covered behavior.

**AC7 — the post-create re-read no longer fakes a channel, and that is
observed.** `storage/src/post_service.rs`'s re-read after `create_rendered_post`
uses `ViewerIdentity::Local { user_id }`, with no placeholder id and no channel
lookup. A **new** `#[apply(backends)]` test drives `perform_post_creation` with
non-public targeting (`audiences: vec![]`) and asserts the created record is
returned — the existing create-path tests all pass `AudienceTarget::Public`
(`storage/src/post_service.rs:581, 614, 646, …, 1115`) and therefore cannot
observe this property.

**AC8 — the dead cache is gone.** `storage::local_channel_id` (free fn),
`static LOCAL_CHANNEL_ID`, and the test
`local_channel_id_returns_the_seeded_local_channel` no longer exist.
`SubscriptionStorage::local_channel_id()` still exists, and
`web/src/subscriptions/api.rs` still uses it at `subscribe` and `unsubscribe`
but no longer at `is_subscribed`.

**AC9 — `is_subscriber` handles all three variants.**
`storage/src/subscriptions.rs:203-222` matches `Local`, `Remote` and
`Anonymous`. Both backends gain an `IS_ACTIVE_LOCAL_SUBSCRIBER` const (2 binds,
channel resolved by subquery) used for `Local`; `IS_ACTIVE_SUBSCRIBER` is
unchanged and used for `Remote`. Existing `is_subscriber` behavior for local
viewers is preserved (`server/tests/storage/mod.rs:293-390` still passes).

**AC10 — the AtomPub DI seam is gone.** `owner_viewer` is infallible and takes
no `subscriptions` argument; `Extension<Arc<dyn SubscriptionStorage>>` no longer
appears in `server/src/lib.rs` or `server/src/atompub/posts.rs`; the router
comment at `server/src/lib.rs:48-51` is removed. All AtomPub owner-post tests
still pass.

**AC11 — stale documentation is corrected.** No doc comment or ADR still
describes the removed shapes: `web/src/viewer.rs:7-15, 28-30, 39-41` (module doc
naming `ViewerIdentity::Channel`, `account_viewer`, `local_channel_id`, and the
fail-closed contract), `storage/src/posts.rs:1930-1972` (the five-placeholder
doc block) and its per-call-site bind comments
(`:1312, 1343, 1384, 1413, 2312, 2322-2323, 2334, 2345-2346`, which state fixed
`$n..$n+4` ranges that are now variant-dependent), ADR-0020's viewer clause, and
ADR-0063's example.

**AC12 — the mock follows.** `web/src/posts/api.rs:935-951`'s
`MockSubscriptionStorage` + `expect_local_channel_id()` in `mutation_owner`
exists only because the update path resolved a viewer through the channel
lookup; it is removed or reduced accordingly, and the affected tests still pass.

**AC13 — the gate is green.** `cargo xtask validate` passes, including the
coverage policy and all four `{sqlite,postgres}×{chromium,firefox}` e2e combos.

## Out of scope

- Constructing any `Remote` viewer in production (Layer B/C work).
- Issue #342 itself — the trait method and the subscription write path. The ship
  step comments on it; that is a process step, not an acceptance criterion.
- Validating that a `Remote` viewer's `channel_id` names a real, non-local
  channel row; no producer exists yet.
