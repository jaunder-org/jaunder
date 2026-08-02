# Batch per-tag write loops — spec (issue #771)

Status: approved (2026-08-01) Issue:
[#771](https://github.com/jaunder-org/jaunder/issues/771) Governing ADR:
[ADR-0092](../../adr/0092-sqlite-bounded-write-lock-occupancy.md) (bounded
write-lock occupancy), which names #771 as remaining work Origin: #766
write-loop audit (`docs/archive/2026-07-31-issue-766-sqlite-busy-e2e-spec.md`,
Deliverable 5) Spun off:
[#784](https://github.com/jaunder-org/jaunder/issues/784) — whether
`MAX_TAGS_PER_POST` earns its keep

## Problem

Three production sites issue one autocommit DB write per tag. Each `tag_post`
(`storage/src/sqlite/posts.rs:144-210`, `storage/src/postgres/posts.rs:134-187`)
is a **whole transaction** — post-exists check, insert tag, select tag_id,
insert post_tags — so a 25-tag post takes 25 write-lock acquisitions and ~100
statements. `untag_post` (`sqlite:217-225`, `postgres:194-202`) is a single
autocommit `DELETE` — not a transaction, but still one write-lock acquisition
per removal.

ADR-0092 prohibits exactly this: _"No per-row write loops … Per-item autocommit
write loops are prohibited in production code, whatever the layer."_

## What the call sites actually want

Every production write site is the same operation — **make this post's tags
equal this set** — and every one already holds the full desired list. **None
holds a diff**; the diff is computed inside a helper, from a read that helper
performs.

| Site                                                    | Path           | Today                                                                                |
| ------------------------------------------------------- | -------------- | ------------------------------------------------------------------------------------ |
| `web/src/posts/api.rs:221-223`                          | web create     | `for label in &validated_tags { tag_post }` (new post, so pure add-all)              |
| `web/src/posts/api.rs:352` → `storage/src/posts.rs:424` | web update     | `apply_post_tag_diff`: read → diff → loop adds → loop removes                        |
| `server/src/atompub/posts.rs:428`                       | AtomPub create | `apply_categories` — same algorithm, hand-inlined duplicate — on a post with no tags |
| `server/src/atompub/posts.rs:530`                       | AtomPub update | `apply_categories`                                                                   |

So one declarative call replaces all four, and `apply_categories`' duplication
(the issue's DRY complaint) disappears rather than being folded.

A sweep of every `for … { … .await … }` in `web/src`, `server/src` and
`storage/src` confirms these are the **only** un-batched per-row write loops
left in production: the others are already inside an open transaction
(`storage/src/posts.rs:2202` audiences, `:2252` media, the backup writers,
`feed_events.rs:270`) or already chunked (`server/src/feed/worker.rs:108`).

## Defect found during design: AtomPub's tag count is unbounded

`entry_to_post_fields` (`server/src/atompub/mapping.rs:105-109`) collects
categories with `filter_map(…).collect()` — **no cap, no dedupe** — while web
routes through `common::tag::parse_and_validate_tags`, which enforces
`MAX_TAGS_PER_POST` (25) and dedupes by canonical slug. An AtomPub client can
therefore POST 10,000 `<category>` elements and drive 10,000 autocommit writes
on a request path.

Batching alone would _not_ fix it: ADR-0092 requires batches be _"capped by
construction … never unbounded input"_, so one transaction holding 10,000
inserts trades many short holds for one very long one. It is therefore in scope
here.

The **dedupe** half is correctness, not policy: `post_tags` has
`UNIQUE(post_id, tag_id)`, so `<category term="Rust"/>` plus
`<category term="rust"/>` is one slug twice. Whether the _cap_'s value of 25 is
justified is a separate product question — deliberately not settled here,
tracked as #784.

## Decisions

**D1 — One declarative storage call.**
`set_post_tags(post_id, desired: &[TagLabel]) -> Result<(), TaggingError>` on
`PostStorage`, implemented per-dialect on `PostDialect`. One transaction
performs: serialize/exists-check → read existing → diff → apply the delta.

**D2 — Diff internally; do not truncate-and-recreate.** `post_tags` carries no
timestamps or surrogate key, so recreating loses no persisted data — but
truncation would break two things. It would let incoming casing win, whereas
`post_tag_diff` documents (`storage/src/posts.rs:307-309`) that _"re-applying an
existing tag with different display casing is a no-op (the existing row's casing
is preserved by storage)"_; and it would turn every tag-untouched post edit into
N deletes + N inserts, cutting against the very goal. Diffing leaves unchanged
rows untouched, so casing preservation and no-write-when-unchanged both fall out
for free.

**D3 — Serialize the read-modify-write per backend, using each dialect's
existing precedent.** SQLite takes `BEGIN IMMEDIATE` up front (today's
`tag_post` pattern, `sqlite/posts.rs:158`), so the read happens under the write
lock and there is no deferred shared→reserved upgrade (ADR-0021). Postgres takes
`SELECT … FROM posts WHERE post_id = $1 FOR UPDATE` (today's `update_post`
pattern, `postgres/posts.rs:52`), which doubles as the existence check.

_This reverses the direction recorded on the issue._ The scope-addition comment
on #771 argued the read must be **hoisted out** so the batched write opens with
nothing before it. That reasoning assumed a deferred `BEGIN`; under
`BEGIN IMMEDIATE` the write lock is already held, so an in-transaction read is
not an upgrade and ADR-0021 is satisfied. Keeping the read inside is strictly
better, because today's read runs as a **separate autocommit read before** the
writes — a TOCTOU window a concurrent tagger can slip through. The issue comment
is corrected as part of this work so a later reader does not mistake the
divergence for an implementation error.

**D4 — Writes are idempotent.** `INSERT … ON CONFLICT DO NOTHING` (Postgres) /
`INSERT OR IGNORE` (SQLite) on `post_tags`; deletes ignore `rows_affected`.
Postgres runs READ COMMITTED, so without this a concurrent `set_post_tags` could
still collide inside the transaction; with it the operation is race-benign and
naturally idempotent. This is also what makes a duplicate slug within `desired`
safe — `post_tag_diff` does **not** dedupe its input
(`storage/src/posts.rs:320-323`), so `["Rust","rust"]` yields two `to_add`
entries for one slug; the second insert is a silent no-op and first-casing wins,
matching `parse_and_validate_tags`. `set_post_tags` therefore needs no dedupe of
its own.

**D5 — `tag_post` and `untag_post` are deleted** from `PostStorage`,
`PostDialect`, both dialect impls, and the mock. `set_post_tags` becomes the
only tag-write API, so the per-row loop cannot regrow — there is nothing left to
loop.

**D6 — `TaggingError::{AlreadyTagged, TagNotFound}` are deleted.** Under D4 they
are unreachable, and they existed only to serve the deleted primitives.
`PostNotFound` (still raised by D3's existence check) and `Internal` remain. No
variant is individually surfaced to users — both front-ends map `TaggingError`
wholesale to an internal-class error (`server/src/atompub/mod.rs:226-232`,
`storage/src/posts.rs:350-359`) — but the deletion does orphan assertions; see
test plan item 5.

**D7 — `apply_post_tag_diff` and `apply_categories` are deleted; `post_tag_diff`
survives.** The pure diff function is the reusable core and keeps its own unit
test (`storage/src/posts.rs:2702`); it is demoted from `pub` to `pub(crate)` and
called inside `set_post_tags`. What goes is the two _apply-loops_ wrapped around
it, one of which was a duplicate of the other.

**D8 — `get_tags_for_post` is deleted, including its ~56 test uses.** D7 removes
its two in-helper callers and D10 the other two, leaving zero **production**
callers — but roughly 56 occurrences across ~42 test functions remain, in
`server/tests/storage/mod.rs`, `server/tests/web/web_posts.rs`,
`server/tests/misc/backup_fixture.rs`, `server/tests/atompub/`, and
`storage/src/posts.rs`'s own test module. All are rewritten to
`get_post_by_id(id, &ViewerIdentity::Anonymous).await?.expect(…).tags`, which
returns the same data slug-ordered (#772). This is the removal identified during
#772 and deferred here, as recorded on this issue.

**D9 — AtomPub routes categories through `parse_and_validate_tags`, at the
handlers.** `entry_to_post_fields` is deliberately **infallible**
(`server/src/atompub/mapping.rs:83-125`, contract stated at `:98-104`), so
validation runs at the two handlers (`server/src/atompub/posts.rs` create
≈`:390`, update ≈`:500`) rather than inside the mapper. This requires a new
`From<TagValidationError> for HandlerError` producing a 4xx — today
`TagValidationError` only bridges to `host::error::InternalError`
(`host/src/error.rs:394`), which AtomPub does not use, so nothing currently
yields AC8's rejection.

This narrows AtomPub's otherwise-lenient category handling: R5 drops a
_malformed_ term rather than failing the entry, and that stays true
(`docs/atompub-marsedit-acceptance.md:62-63` is about malformed terms, not
over-cap, so it is not contradicted). What changes is that an _over-cap_ entry
now fails cleanly instead of silently driving an unbounded batch.

**D10 — Web derives feed-event slugs locally.** Both web sites re-read tags
purely to harvest slugs for the feed-event fan-out. `tag_post` stores exactly
`TagLabel::slug()` (`common/src/tag.rs:85`), so the slugs are already
determined: create uses `validated_tags`, update unions `old_tag_slugs` (bound
at `web/src/posts/api.rs:312` from the `get_post_by_id` at `:309`) with the new
desired slugs. No extra read.

**D11 — An empty desired set means "remove all tags", not "do nothing".**
`enqueue_many` early-returns on empty input
(`storage/src/feed_events.rs:261-263`); `set_post_tags` must **not** copy that
guard, because `set_post_tags(id, &[])` on an existing post is a meaningful
instruction to clear its tags. Called on a fresh post it is a harmless no-op
that writes nothing (the diff is empty).

**D12 — `set_post_tags` does not itself cap its input; the bound holds by
construction of the call graph.** ADR-0092 requires batches be capped by
construction. After D9 the only production callers are web and AtomPub, and
_both_ now pass through `parse_and_validate_tags`, so no production path can
hand `set_post_tags` an unbounded set. Storage stays policy-free (the cap lives
in `common::tag` with the rest of tag policy), which also lets test fixtures
deliberately exceed it — see test plan item 6.

**D13 — Soft-deleted posts: behaviour unchanged.** Today's `tag_post`
exists-check does not filter `deleted_at` (`sqlite/posts.rs:162`), unlike
`update_post` (`:53-54`). `set_post_tags` keeps that: no `deleted_at` filter, so
tagging a soft-deleted post continues to succeed exactly as now. Called out
because D3's Postgres `FOR UPDATE` doubles as the existence check and could
silently change it.

**D14 — No new ADR.** ADR-0092 already governs and explicitly names #771; D1–D4
are implementation choices inside it, not new decisions. ADR-0068's live
enumeration of label-carrying sites
(`docs/adr/0068-tag-identity-label-split.md:50-51`) names `tag_post` and gets a
consequence note; ADR-0021 and ADR-0063 mention it only as historical narrative
and are left alone.

## Acceptance criteria

- **AC1 — One write-lock acquisition per tag mutation.** Setting a post's tags
  issues exactly one `set_post_tags` call. Pinned by mock-counted tests in the
  style ADR-0092 already uses: `times(1)` on the tag-setting paths, and
  **`times(0)` when tags are not being changed** — `web/src/posts/api.rs:351`
  only writes when `new_tags` is `Some` (`server/tests/web/web_posts.rs:2257`
  covers that path).
- **AC2 — All four sites go through the one call.** `web/src/posts/api.rs`
  (create, update) and `server/src/atompub/posts.rs` (create, update) each call
  `set_post_tags` once; `apply_post_tag_diff`, `apply_categories`, `tag_post`,
  `untag_post` and `get_tags_for_post` no longer exist anywhere in the tree,
  tests included (D5, D7, D8).
- **AC3 — Display casing is preserved.** Setting tags on a post that already
  carries a tag with the same slug but different casing leaves the stored
  `tag_display` unchanged. Both backends.
- **AC4 — Unchanged tags cause no writes.** Calling `set_post_tags` with the
  post's current set leaves the existing rows **physically untouched**, asserted
  on real row identity: `ctid` (Postgres) and `rowid` (SQLite), not on column
  values — a DELETE+INSERT reproduces `tag_id`/`tag_display` exactly, so a
  column-value check would pass the very truncate-and-recreate D2 forbids.
  **SQLite trap:** `post_tags` has no `AUTOINCREMENT`, so rowid is
  `max(rowid)+1` and deleting the table's highest rows can hand the same rowids
  back; the fixture must seed a second post whose tags occupy higher rowids so
  reuse is impossible.
- **AC5 — Add, remove and clear all work in one call.** A desired set that adds
  some tags and drops others reaches exactly that state; `&[]` clears all tags
  (D11). Both backends.
- **AC6 — Idempotent and race-benign.** Calling `set_post_tags` twice with the
  same desired set succeeds both times and leaves one row per tag; a `desired`
  containing two labels with the same slug also yields one row, with
  first-occurrence casing (D4).
- **AC7 — A missing post is still rejected.** `set_post_tags` on a nonexistent
  post returns `TaggingError::PostNotFound`; a soft-deleted post is still tagged
  successfully (D13). Both backends.
- **AC8 — AtomPub is bounded.** An entry with more than `MAX_TAGS_PER_POST`
  distinct categories is rejected with a **4xx** (requiring the new
  `TagValidationError → HandlerError` bridge, D9) rather than written; duplicate
  categories are deduped; a _malformed_ term is still skipped leniently, not
  rejected.
- **AC9 — Feed events still fan out to the right surfaces.** Creating and
  updating a post with tags enqueues feed events for the same surfaces as
  before, with slugs derived rather than re-read.
- **AC10 — The decode gate stays green.**
  `xtask/src/steps/sqlx_newtype_decode_check.rs:597-614` holds two ALLOWLIST
  entries keyed on `function: "tag_post"`, and a stale entry is a hard failure
  (`:1557`, test at `:2203`). Both are updated: SQLite keeps a `bool` COUNT
  decode (renamed to `set_post_tags`), Postgres's `SELECT … FOR UPDATE` changes
  the decode target, so its entry is rewritten or removed to match.
- **AC11 — Docs naming the deleted items are corrected**, including the rustdoc
  intra-doc links that would otherwise break the `doc-links` gate: the
  `PostDialect` rationale (`storage/src/posts.rs:806-812`, which links
  `[tag_post]`/`[untag_post]` — its `INSERT OR IGNORE` vs `ON CONFLICT` half
  carries over to `set_post_tags`, its `rows_affected` half is discarded by D4,
  so this is a rewrite not a rename), `storage/src/posts.rs:895-896`,
  `PostTagDiff`'s doc (`:293-294`, which claims callers do the writes),
  `server/src/atompub/mapping.rs:102`, and `common/src/test_support.rs:441`.
- **AC12 — Backend parity.** `set_post_tags` is implemented on both dialects and
  every behavioural criterion above is asserted on both.
- **AC13 — The gate is green.** `cargo xtask validate` passes.

## Test plan

Per `CONTRIBUTING.md` "Backend parity rules", every DB-touching test is
backend-explicit, so the behavioural criteria are dual-backend by construction.

1. **New `set_post_tags` behaviour tests** (`storage/src/posts.rs`,
   `#[apply(backends)]`): add/remove/clear (AC5), casing preservation (AC3),
   no-physical-write-when-unchanged via `ctid`/`rowid` with the higher-rowid
   decoy post (AC4), idempotence and duplicate-slug input (AC6), missing post
   and soft-deleted post (AC7).
2. **Mock-counted call tests** (AC1): `times(1)` on the tag-setting paths,
   `times(0)` on the tags-unset update path, mirroring ADR-0092's existing
   `enqueue_many` guards.
3. **AtomPub bounding tests** (AC8): over-cap entry → 4xx; duplicate categories
   deduped; malformed term still skipped.
4. **Rewrite the three primitive-specific storage tests** that D5 orphans:
   `tag_post_insert_error_returns_internal` (`storage/src/posts.rs:3330`),
   `apply_post_tag_diff_adds_then_removes_tags` (`:3561`),
   `tag_post_round_trips_slug_and_label` (`:3605`) — re-express each against
   `set_post_tags`. `post_tag_diff_adds_removes_keeps` (`:2702`) is pure and
   unaffected.
5. **Resolve the tests that pin now-deleted behaviour** (D6). Each is **deleted,
   with the behaviour it pinned named in the commit message** — the behaviour is
   gone, not merely unasserted: `retag_same_post_with_same_tag_fails`
   (`server/tests/storage/mod.rs:3394`), `duplicate_tag_error` (`:3940`),
   `tag_post_multiple_attempts` (`:4433`), `untag_nonexistent_post` (`:3428`),
   `untag_nonexistent_tag_error` (`:4142`), `get_tags_nonexistent_post`
   (`:3439`), plus the three `TaggingError` Display/Debug unit tests
   (`storage/src/posts.rs:2726`, `:2732`, `:2738`) and the AtomPub status test
   naming `AlreadyTagged` (`server/src/atompub/mod.rs:423`). Idempotence (AC6)
   is the replacement for the first three — the new behaviour where the old
   error was.
6. **Convert every test write loop** to `set_post_tags` — `SeedRawPost::create`
   (`storage/src/test_support.rs:1066`), `server/tests/web/web_tags.rs:108` (60
   tags) and `:134` (20), and the per-row seed loops in
   `server/tests/storage/mod.rs`. The `web_tags` fixtures deliberately exceed
   `MAX_TAGS_PER_POST` to exercise `list_tags` clamping; they bypass the
   front-end validation door and D12 keeps storage policy-free, so they convert
   unchanged.
7. **Rewrite the ~56 `get_tags_for_post` read sites** (D8) to
   `get_post_by_id(…).tags`, across `server/tests/storage/mod.rs` (~56
   occurrences in ~42 fns), `server/tests/web/web_posts.rs:2004,2113,2301`,
   `server/tests/misc/backup_fixture.rs:201`,
   `server/tests/feed/feed_regenerate.rs:237` (a doc comment), and
   `storage/src/posts.rs:3580,3594,3622`. The AtomPub test files contain no
   `get_tags_for_post`; their `tag_post` writes belong to item 6.
8. **No server-fn coverage regeneration is needed.** An earlier draft of this
   item claimed the evidence is text-identity anchored to server-fn bodies; it
   is not. The artifacts are keyed on `#[macros::server]` fn _names_ plus e2e
   trace/test _titles_ (`xtask/src/server_fn_coverage/io.rs:33-52`, `:56-66`).
   This change adds and removes no server fn and renames no test, so nothing
   regenerates. (Commit `cba25194` regenerated because a test title changed.)

## Out of scope

- The value of `MAX_TAGS_PER_POST` → **#784**. This issue applies the _existing_
  cap at the AtomPub door so both front-ends behave alike; it takes no position
  on 25.
- `list_tags`, `list_posts_by_tag`, `list_user_posts_by_tag` — read-only,
  untouched.
- Session-touch write amplification (#770), the other occupancy item ADR-0092
  tracks.
