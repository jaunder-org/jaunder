# Issue #711 — a post→media reference table, written at render time

**Status:** approved design, pending implementation **Issue:**
[#711](https://github.com/jaunder-org/jaunder/issues/711) (Bug, P1, milestone
"Correctness & data integrity") **Blocked by:** #720 (percent-encoded filename
canonical) — **merged**, at the branch's fork point `1623e548`.

## Problem

No relation records which posts reference which media, so every question about
media usage is answered by substring-searching post bodies.
`web/src/media/api.rs` searches for the _exact_ derived serve URL across at most
1000 published posts and 1000 drafts. Three consequences:

1. **Wrong answers.** A post addressing the same file with a different spelling
   — the raw filename rather than the percent-encoded one, or the AtomPub member
   URL, which shares none of the serve URL's prefix — does not match, producing
   a false "not referenced" and bypassing the guard `force` exists to provide.
2. **Truncation.** A reference in post 1001 is invisible.
3. **Does not scale to the question that matters.** Orphan reclamation asks "is
   this on-disk entry referenced by any post?", which a substring scan can only
   answer by reading every corpus, per file.

The damage is _not_ data loss: nothing in the tree deletes a stored media file,
so the post keeps rendering. The damage is a wrong answer, plus a record that
vanishes from the media library and from quota accounting
(`get_user_upload_usage` sums rows).

## Design

Add a `post_media` join table, written when a post is rendered.

### D1 — the reference set is always _derived_, and publication is not an edit

`common::render::render` (`common/src/render.rs:341`) is the single rendering
chokepoint, with exactly three production callers, all in
`storage/src/post_service.rs` (79, 169, 331).

**But rendering is not currently the only way `rendered_html` reaches storage.**
`web/src/posts/api.rs:507` (`publish`) writes an `UpdatePostInput` built from
the _stored_ record — `rendered_html: existing.rendered_html` — and deliberately
does not re-render, because publishing a draft must not alter its content. It is
the most common write in the product, and it is the **only** non-rendering
production construction site of either input type; every other is `post_service`
or a test fixture.

That one exception makes the naive form of this design actively destructive. If
`render` returned a reference set and callers _supplied_ it, `publish` would
have none to supply, would pass an empty one, and — since D8 replaces a post's
rows on update — **publishing a draft would delete every `post_media` row it
had.**

**The fix is to stop routing publication through the update path.** Publication
changes one timestamp; it is not an edit. A dedicated storage operation

```rust
async fn publish_post(
    &self,
    post_id: PostId,
    user_id: UserId,
) -> Result<PostRecord, UpdatePostError>;
```

sets `published_at = COALESCE(published_at, now)` under the same ownership and
not-deleted checks `update_post` already performs, and touches nothing else —
not the body, not `rendered_html`, not the slug, not audiences, not
`post_media`.

This is cheaper as well as safer. Today's `publish` reads the post, then reads
its audiences (`:505`) purely so the update won't clobber them, then rewrites
them; the dedicated operation needs none of that, and never touches the rows it
has no business touching.

With publication out of the update path, **every remaining production writer
renders**, so the invariant needs no escape hatch:

```rust
pub struct RenderOutput {
    pub html: RenderedHtml,
    media: Vec<MediaRef>,   // private — no caller can supply or desynchronise it
}

impl RenderOutput {
    pub fn render(body: &PostBody, format: &PostFormat) -> Self;  // the only constructor
    pub fn media(&self) -> &[MediaRef];
}
```

`CreatePostInput` and `UpdatePostInput` **replace** their
`rendered_html: RenderedHtml` field with `rendered: RenderOutput`. A value whose
reference set disagrees with its HTML is unrepresentable, because the set is
derived from the HTML by the sole constructor and cannot be written by anyone.

An earlier draft of this design instead gave `RenderOutput` a second constructor
that adopted already-rendered HTML and re-derived the set from it, letting
`publish` keep using `update_post`. That was rejected: it re-tokenises the whole
stored body on the product's busiest write, only to recompute rows that are
already correct, and then delete-and-reinsert them identically — real work, and
row churn, to paper over a call site that should not have been calling
`update_post` at all. It also left a public door whose only purpose was to serve
that misuse.

AtomPub needs no special case — its `collection_post` supplies `body` + `format`
and goes through `perform_post_creation`/`perform_post_update`
(`server/src/atompub/posts.rs:382,505`). Seeders go through `render_post_input`.

The pure `&str → Vec<MediaRef>` extractor stays public in its own right (D6):
the coupling test needs it, and reclamation and any future backfill will reuse
it. It is simply no longer reachable as a way to _construct_ a `RenderOutput`.

### D2 — extract from the rendered, sanitized HTML

Extraction reads the sanitized rendered HTML, not the source AST. That catches a
raw `<img>` embedded in a Markdown or Org body (both parsers pass raw HTML
through untouched), and excludes anything sanitisation strips.

**What counts as a reference: a URL, in a position where the post points a
reader at it, that _names_ a stored media entry.** An earlier formulation —
"this media will actually load" — is not quite honest, because D4 counts the
AtomPub member URL, which returns an Atom entry document and requires a matching
authenticated user (`server/src/atompub/media.rs:146,152`); embedded in an
`<img>` it would not load at all. Naming is the property that matters for both
consumers: the delete guard asks "would removing this record contradict
something a post says", and reclamation asks "does anything name this on-disk
entry".

`cite` (on `blockquote`/`del`/`ins`/`q`) is excluded **as a deliberate scope
call**, not as a consequence of the principle: it is provenance metadata about
where a quote came from, not a pointer the reader can follow, and no browser
fetches, displays, or navigates it. A9 makes that exclusion explicit and
reviewable rather than implicit.

Verified against `ammonia-4.1.4/src/lib.rs:376`, the only URL-bearing attributes
the `SANITIZER` allowlist permits are `a[href]`, `img[src]`, and those `cite`
attributes. `video`, `audio`, `source` and `track` are not in the tag allowlist;
`srcset` is not among `img`'s attributes; `area` is an allowed tag with no
allowed attributes.

**Extraction surface: `img[src]` and `a[href]`.**

**Deliberate narrowing:** the current guard also substring-searches the _source
body_ (`web/src/media/api.rs:170`). The new design reads rendered HTML only, so
a URL that survives in the body but not the rendered output — inside a fenced
code block, say — stops counting. That is correct under the principle above: a
URL displayed as literal text points nobody at anything, and deleting that media
breaks nothing.

### D3 — the sanitiser/extractor coupling test

Separating the extractor's surface from the sanitiser's allowlist recreates this
issue's own failure mode: widen the sanitiser later, forget the extractor, and
the table silently acquires a blind spot.

The obvious test — "enumerate the permitted pairs, compute which are
URL-bearing, assert those are covered" — has a hole exactly where the danger is.
ammonia's `is_url_attr` (`ammonia-4.1.4/src/lib.rs:2531`) is a **private** free
function, so the test would need its own hand-written URL-attribute predicate;
and a hand-written predicate would not recognise `srcset` as URL-bearing, which
is the one attribute follow-up issue #1 flags as hazardous.

**So the assertion is inverted.** Enumerate every permitted
`(element, attribute)` pair — which is
`tags × generic_attributes ∪ tag_attributes`, since `generic_attributes` applies
to every tag — and require each to appear in **either** the extractor's pair
list **or** an explicit `KNOWN_INERT` list. Any newly permitted pair fails the
test until a human classifies it. No URL-attribute predicate is needed, and
nothing can be admitted silently. `clone_tags`, `clone_tag_attributes` and
`clone_generic_attributes` are public on `ammonia::Builder`, so the enumeration
half is straightforward.

This is what makes video/audio embedding safely separable (see "Follow-up
issues").

**Both halves are declarative tables, and the walk is driven by them:**

```rust
/// The (element, attribute) pairs whose values name media. Adding an element to
/// `SANITIZER` means adding its URL-bearing attributes here — the extractor walks
/// this table and knows no tag names of its own.
const MEDIA_URL_ATTRS: &[(&str, &str)] = &[("a", "href"), ("img", "src")];

/// Permitted pairs deliberately *not* treated as media references, with the reason.
/// Present so `sanitizer_surface_is_fully_classified` can tell "considered and
/// excluded" from "nobody looked".
const KNOWN_INERT_ATTRS: &[(&str, &str)] = &[/* cite on blockquote/del/ins/q, … */];
```

Extending to video/audio is then a **data edit** — append `("video", "src")`,
`("video", "poster")`, `("audio", "src")`, `("source", "src")`,
`("track", "src")` — with no change to the walk. The walk takes the table as a
parameter so a test can drive it with a synthetic pair, which is what proves no
tag name is baked in (A9b).

A multi-URL attribute such as `srcset` does **not** fit this table's shape (one
attribute, one URL) and would need it widened to carry a per-attribute parse
mode. That is the correct outcome: the coupling test forces whoever permits
`srcset` to confront that rather than silently get a wrong single-URL parse.

**Documentation obligations (this is the part that keeps the above true):**

- Each table carries the doc comment above, stating the contract in the place
  someone edits.
- The `SANITIZER` definition (`common/src/render.rs:119`) gains a pointer to
  both tables and to the coupling test. Its comment already says "Widening this
  list is a security decision; `sanitize_*` tests pin both halves" — it must
  also say that widening obliges you to classify the new attributes, because
  that comment is what a person widening the allowlist is looking at.
- The coupling test carries a doc comment explaining what a failure means and
  how to resolve it (classify the pair into one table or the other), so the
  failure is self-servicing rather than a puzzle.
- The decision and its rationale are recorded in the ADR, not only here — a spec
  is archived at ship, an ADR is not.

### D4 — which URLs count

Two layouts are recognised:

| Form               | Shape                                        | Source              |
| ------------------ | -------------------------------------------- | ------------------- |
| Serve URL          | `/media/<source>/<p1>/<p2>/<sha>/<filename>` | carried in the URL  |
| AtomPub member URL | `/atompub/<username>/media/<sha>/<filename>` | implicitly `Upload` |

The AtomPub member handlers (`server/src/atompub/media.rs:161` and `:187`)
hardcode `MediaSource::Upload`, so the member form pins `source` without
carrying it.

- **Query strings and fragments are stripped** before matching, so
  `…/photo.jpg?v=2` counts.
- **`p1`/`p2` must match the hash's leading bytes.** `server/src/media.rs:236`
  404s a mismatch, so such a URL names nothing.
- **Scheme, host and port are ignored entirely.** Any URL whose _path_ parses as
  one of the two layouts is a reference, wherever it claims to live.

Host-blindness is deliberate. `render` is a pure function with no access to
configuration; threading a site host into it would change every caller, make
rendering config-dependent, and — because `rendered_html` is _stored_ — mean a
later hostname change silently invalidates what was already extracted. The
failure this admits is benign in both directions: for the delete guard, a
foreign-host URL carrying a valid 64-hex digest _and_ a matching filename
refuses a delete that could have proceeded, which `force` covers; for
reclamation it leaves a file unreclaimed rather than deleting a live one. Both
err toward "referenced". Host-aware matching is filed as a follow-up.

### D5 — filename normalisation reuses the read path's door

The serve route already solves raw-vs-encoded: axum percent-decodes the path
segment and `SoftPath<ProfferedFilename>` re-encodes it to recover the stored
spelling
(`resolve_media_path_recovers_the_stored_spelling_from_a_decoded_segment`,
`server/src/media.rs:418`).

The extractor feeds each URL's filename segment through **the same door** —
percent-decode, then `ProfferedFilename` → `Filename` — so `my%20photo.jpg` and
raw `my photo.jpg` converge on one canonical `Filename`. This is a normalisation
on the write path, once, not a transform at a comparison point (the bug class
#675, #708 and this issue are all instances of). With #720 the join is then byte
equality.

**Known limit, accepted:** decode-then-encode is not injective for a filename
whose own characters spell an escape. A file literally named `a%20b.jpg` is
stored canonically as `a%2520b.jpg`; a post embedding the raw spelling
`a%20b.jpg` decodes to `a b.jpg` and canonicalises to `a%20b.jpg`, matching a
_different_ entry. This is inherent to accepting raw spellings at all, and is
documented rather than fixed.

### D6 — where the code lives

`common/src/media.rs` states that `media_path` is "the **single** definition of
that layout". A URL→triple parser is the exact inverse of `media_url`
(`common/src/media.rs:647`), so it lives directly beside it:

- **`common::media`** (ungated): `MediaRef { source, sha256, filename }` and the
  URL parser. Pure string work, no HTML, no `sanitize` gate — cheaply
  unit-testable and directly reusable by the reclamation issue. A round-trip
  property test (`parse(media_url(x)) == x`) pins parser and formatter together.
- **`common::render`** (gated on `sanitize`, with `render`): `RenderOutput` and
  the HTML walk, which tokenises the sanitized string and hands each
  `img[src]`/`a[href]` value to that parser. `storage/Cargo.toml:12` enables
  `common`'s `sanitize` feature, so a gated `RenderOutput` is nameable from
  `CreatePostInput`.

`RenderOutput` rather than `RenderedPost`: `storage::post_service` already has
`RenderedPostContent` (`:29`) and `RenderedPostUpdate` (`:128`), which hold
_unrendered_ body + format. A `RenderedPost` sitting beside
`RenderedPostContent`, meaning the opposite side of the same operation, is a
name collision waiting to confuse.

**The walk re-parses ammonia's output** rather than collecting during the clean
pass. Collecting inside `attribute_filter` avoids a second parse, but ammonia
permits only one attribute filter — the existing one already enforces the
`language-*` class policy and would have to grow a second unrelated job — and it
runs at a point whose ordering against URL-scheme filtering would have to be
verified and then depended on. Re-parsing the final string is the literal
reading of "extract from the rendered, sanitized HTML", has no ordering
subtleties, and yields a pure `&str → Vec<MediaRef>` function that the coupling
test and future reclamation work reuse directly.

Parsing uses **`html5ever`'s tokenizer**, not its tree builder: only start tags
and their attributes are needed. `html5ever` 0.39.0 is already in `Cargo.lock`
via ammonia, so a direct dependency on `0.39` version-matches and reuses the
existing Nix vendor rather than forcing a rebuild.

### D7 — schema

Per-dialect DDL, as the repo already does for `post_audiences`:

`storage/migrations/sqlite/0025_create_post_media.sql`

```sql
CREATE TABLE post_media (
    post_id  INTEGER NOT NULL REFERENCES posts(post_id),
    source   TEXT NOT NULL,
    sha256   TEXT NOT NULL,
    filename TEXT NOT NULL,
    PRIMARY KEY (post_id, source, sha256, filename)
);
CREATE INDEX idx_post_media_lookup ON post_media(sha256, filename, source);
```

`storage/migrations/postgres/0025_create_post_media.sql`

```sql
CREATE TABLE post_media (
    post_id  BIGINT NOT NULL REFERENCES posts(post_id) DEFERRABLE INITIALLY IMMEDIATE,
    source   TEXT NOT NULL,
    sha256   TEXT NOT NULL,
    filename TEXT NOT NULL,
    PRIMARY KEY (post_id, source, sha256, filename)
);
CREATE INDEX idx_post_media_lookup ON post_media (sha256, filename, source);
```

Both dialect details are load-bearing, not cosmetic:

- `posts.post_id` is `BIGSERIAL` on Postgres
  (`storage/migrations/postgres/0008_create_posts.sql:2`), so an `INTEGER` child
  column is a hard error ("Key columns are of incompatible types").
- `0024_defer_foreign_keys.sql` was a one-shot pass over then-existing
  constraints, so a new FK must declare its own deferrability.
  `every_foreign_key_is_deferrable` (`storage/src/postgres/schema.rs:19`) fails
  otherwise — and it matters at runtime: `backup_table_set` sorts alphabetically
  (`storage/src/backup.rs:33`) and `"post_media" < "posts"`, so restore loads
  the child before the parent and depends on `SET CONSTRAINTS ALL DEFERRED`
  (`storage/src/postgres/backup.rs:97`).

Further notes:

- **No `user_id` column** — `post_id` already determines the author. Storing the
  table user-agnostically means the "referenced by anyone?" query reclamation
  needs is a plain existence check with no later schema change.
- **FK on `post_id` only.** No FK to `media`, whose primary key is
  `(user_id, sha256, filename, source)` and so cannot be referenced by a
  URL-derived triple. A post may legitimately reference media that has no row.
- The PK deduplicates a post embedding the same image twice; the extractor
  returns a deduplicated, sorted `Vec`, so no dialect-divergent conflict
  handling is needed and the extractor's output is deterministic.
- Posts are only ever **soft**-deleted (`soft_delete_post`,
  `storage/src/posts.rs:1030`), so rows never orphan and no cascade is required.
- `storage/src/backup.rs:26` auto-discovers tables against a denylist, so
  `post_media` travels with backup; the golden list (`:700-726`) needs the new
  table.

### D8 — the write and read sides

**Write:** a generic `replace_post_media::<DB>` in `storage/src/posts.rs`,
mirroring `replace_post_audiences::<DB>`, called inside the post's existing
`BEGIN IMMEDIATE` transaction in `create_post`/`update_post`. Delete-then-insert
on update, since an edit can remove a reference — which is exactly why D1 must
guarantee every `UpdatePostInput` carries a correct set, and why publication
must not travel this path at all.

**Publish:**
`PostStorage::publish_post(post_id, user_id) -> Result<PostRecord, UpdatePostError>`
per D1 — one
`UPDATE posts SET published_at = COALESCE(published_at, now), updated_at = now`
under the existing ownership / not-deleted checks, plus a re-read of the record
in the same transaction. It writes no child rows of any kind.

**Read:** on `PostStorage`, keeping both halves of `post_media`'s lifecycle in
one trait and one module:

```rust
async fn list_posts_referencing_media(
    &self,
    user_id: UserId,
    media: &MediaRef,
) -> sqlx::Result<Vec<PostId>>;
```

It takes the `MediaRef` rather than three loose arguments — `source`, `sha256`
and `filename` are three adjacent stringish values, exactly the transposition
hazard the newtype rule exists for. Scoped by `user_id`, filtered on
`deleted_at IS NULL`, ordered by `post_id`, **with no limit**.

`web/src/media/api.rs::delete` then calls it instead of scanning; it already
pulls `PostStorage` from context (`:142`), so there is no new DI wiring. The
`RowLimit` (`:152`), the two list calls (`:153`, `:163`) and `viewer_identity`
(`:158`) all go away. `DeleteResult` is unchanged.

**Delete:** the guard decision and the row delete are **one statement**, so
there is no check-then-delete window at all:

```sql
DELETE FROM media
 WHERE user_id = $1 AND source = $2 AND sha256 = $3 AND filename = $4
   AND ($5 OR NOT EXISTS (
         SELECT 1 FROM post_media pm
           JOIN posts p ON p.post_id = pm.post_id
          WHERE p.user_id = $1 AND p.deleted_at IS NULL
            AND pm.source = $2 AND pm.sha256 = $3 AND pm.filename = $4))
RETURNING sha256
```

exposed as

```rust
enum TryDeleteOutcome { Deleted, RefusedReferenced }

async fn try_delete_media(&self, user_id: UserId, media: &MediaRef, force: bool)
    -> Result<TryDeleteOutcome, DeleteMediaError>;
```

with `fetch_optional` giving the authoritative outcome. (A bare `bool` cannot
carry the not-found case the next bullet requires.) A single statement is atomic
in both engines, so no transaction, no locking, and none of the Postgres
isolation-level difficulty a two-step check would need (a concurrent
`post_media` insert cannot slip between two statements when there is only one).

This follows ADR-0021's existing move — a single `UPDATE … RETURNING` replacing
a transaction for feed-event claiming (`storage/src/feed_events.rs:96`) — rather
than inventing a pattern.

Consequences of the shape:

- **The refuse-unless-forced policy moves into storage.** Atomicity requires the
  check and the delete be one statement, so `web/src/media/api.rs::delete`
  becomes pure reporting: call `list_posts_referencing_media` for the message,
  call `try_delete_media` for the decision.
- **The referencing-post list stays a separate, advisory query.** Postgres could
  fold it into the same statement with a data-modifying CTE, but SQLite does not
  support `DELETE` as a CTE term, so that buys one round trip on one backend at
  the price of two divergent implementations of one operation — against
  ADR-0053's parity grain. The list only populates a message and does not need
  to be atomic with the decision.
- **Not-found must stay distinguishable from refused.** Today a missing row
  returns `DeleteMediaError::NotFound` (`storage/src/sqlite/media.rs:41`), which
  the web layer surfaces as an error. The conditional statement returns no row
  in _both_ cases, so the failure path does one existence check on `media` to
  classify. That query is advisory and on the cold path only — the decision was
  already made atomically, so classifying it afterwards cannot reopen the race.
  Today's `NotFound` behaviour is preserved exactly.
- **The dialect method may become unnecessary.**
  `MediaDialect::delete_media_row` exists solely because `.rows_affected()` is
  not callable on the generic `DB::QueryResult`
  (`storage/src/media.rs:131-136`). `RETURNING` + `fetch_optional` is generic,
  so `try_delete_media` can live in the shared `MediaStore` impl and the dialect
  method can go — unless A17e forces the SQLite fallback, in which case it stays
  as the divergence point.

**Verification risk, deliberately not assumed away.**
`storage/src/sqlite/sessions.rs:19` records that "SQLite's RETURNING with a
correlated subquery causes `SQLITE_BUSY` under concurrency", which is why that
call site is two statements in a transaction. That case put the correlated
subquery in the `RETURNING` clause to pull a column from another table; this one
puts it in `WHERE` and returns only `media`'s own column — a different shape,
but _different_ is not _verified_. The plan carries a concurrency test hammering
this statement against concurrent post writes on SQLite. If `SQLITE_BUSY`
appears, the SQLite dialect falls back to the two-statement-in-a-transaction
form (`BEGIN IMMEDIATE` serialises it correctly there); Postgres keeps the
single statement. That fallback is a dialect-level implementation difference
with identical observable behaviour, which ADR-0053 permits — unlike differing
_semantics_.

### D9 — decisions deliberately _not_ taken

- **No backfill.** The table starts empty and fills as posts are created or
  edited. There are no production users, so pre-existing rows do not need
  reconciling.
- **The check-then-delete race is closed**, by D8's single conditional statement
  rather than by a transaction. An earlier draft of this spec accepted the race,
  on the grounds that its worst outcome (a media row gone while a post still
  references it — the file survives on disk and still serves, so the post
  renders) is the same state a deliberate `force` delete produces, and that
  closing it properly would need SERIALIZABLE isolation or media locks taken by
  the post write path. That reasoning was sound but the premise was wrong: it
  assumed two statements. One statement needs neither.
- **Cross-user references do not block a delete.** `media` is keyed per-user, so
  two users can each hold a row for the same on-disk entry. If user B's post
  blocked user A's delete, A would be blocked by content they cannot see, and
  `DeleteResult.referenced_in_posts` would hand A a list of B's post IDs — a
  disclosure the current code does not make. The _table_ is user-agnostic so
  reclamation can ask the broader question later; _this_ issue's guard is scoped
  to the deleting user's own posts.

## Acceptance criteria

Each is stated so a conformance review can tell delivered from not.

**Extraction (unit, `common`)**

- A1.
  `parse_media_url(media_url(source, sha, filename)) == Some(MediaRef { … })`
  for every `MediaSource` and for filenames requiring percent-encoding — a
  round-trip property test against `media_url`.
- A2. Rendering a body that embeds the serve URL with a **raw (unencoded)**
  filename yields the same `MediaRef` as the canonical encoded spelling.
- A3. Rendering a body that embeds the **AtomPub member URL**
  (`/atompub/<username>/media/<sha>/<filename>`) yields a `MediaRef` with
  `source == Upload`.
- A4. A Markdown body containing a raw `<img src="…">` yields the corresponding
  `MediaRef` — the rendered-HTML choice, not the source AST.
- A5. A URL whose `p1`/`p2` do not match the hash's leading bytes yields no
  `MediaRef`.
- A6. A serve URL bearing a query string or fragment still yields its
  `MediaRef`.
- A7. An absolute URL on a foreign host whose path matches a layout yields a
  `MediaRef` (host-blindness, pinned deliberately).
- A8. A body whose only media URL sits in a stripped element (e.g.
  `<video src>`) or in a fenced code block yields no `MediaRef`.
- A9. The coupling test: every `(element, attribute)` pair permitted by
  `SANITIZER` — computed as `tags × generic_attributes ∪ tag_attributes` —
  appears in either `MEDIA_URL_ATTRS` or `KNOWN_INERT_ATTRS`. Adding any pair to
  the sanitiser without classifying it fails this test. The `cite` attributes
  appear in `KNOWN_INERT_ATTRS`, making D2's scope call explicit. Demonstrated
  by temporarily widening the sanitiser in a test and observing the failure —
  the test must be shown to bite, not merely to pass.
- A9b. **Extensibility is falsifiable, not asserted.** The walk takes its pair
  table as a parameter; a test drives it with a synthetic `(element, attribute)`
  pair absent from `MEDIA_URL_ATTRS` and gets that element's URL extracted. This
  fails if any tag name is hardcoded in the walk, and is what makes "adding
  `<video>` is a data edit" a checked claim rather than a hope.
- A9c. The documentation obligations of D3 are met: both tables carry contract
  doc comments; the `SANITIZER` definition names both tables and the coupling
  test; the coupling test's doc comment states what a failure means and how to
  resolve it.
- A10. A `RenderOutput` cannot be built with a caller-supplied reference set:
  the `media` field is private and `render` is its only constructor, pinned by a
  `compile_fail` doctest in the style `common/src/media.rs` already uses for
  `ContentType`.

**Persistence (dual-backend, ADR-0053 — sqlite and postgres)**

- A11. Creating a post that embeds media writes the matching `post_media` rows.
- A12. Editing a post to remove a media reference removes its row; editing to
  add one adds it.
- A13. A post referencing nothing writes no rows — no false positives.
- A14. Rows are written for posts created via **both** the web path and the
  AtomPub path.
- A15. **Publishing a draft that references media leaves its `post_media` rows
  intact.** Regression cover for D1's hazard; fails against any design that
  routes publication through `update_post`.
- A15b. `publish_post` changes only `published_at` and `updated_at`: body,
  format, `rendered_html`, slug, summary, audiences and `post_media` rows are
  byte-identical before and after, and an already-published post keeps its
  original `published_at`.
- A15c. `publish_post` rejects a post owned by another user, and a soft-deleted
  post, with the same errors `update_post` returns today.
- A16. `list_posts_referencing_media` returns only the given user's posts,
  excludes soft-deleted posts, and returns post IDs in ascending order.
- A17. **No truncation:** with more than 1000 posts seeded for a user, a
  reference in a post beyond the old 1000-row window is returned. Seeded via the
  batching `create_posts`, so this is one round trip per backend, not 1200.
- A17b. `try_delete_media` refuses (row still present) when a non-deleted post
  of that user references the media and `force` is `false`; deletes when `force`
  is `true`; and deletes when nothing references it.
- A17c. `try_delete_media` still returns `DeleteMediaError::NotFound` when no
  such media row exists, distinguishing it from a refusal even though the
  conditional statement returns no row in both cases. Pins that today's
  not-found behaviour survives the rewrite.
- A17d. **The guard is atomic.** Under sustained concurrent post writes that add
  and remove references to the same media, repeated
  `try_delete_media(force = false)` never deletes a row while a live reference
  exists. Run against both backends; this is the criterion the single-statement
  design exists to satisfy, and it fails against any two-step check-then-delete.
- A17e. **No `SQLITE_BUSY` regression.** The concurrency exercise in A17d
  completes on SQLite without a `SQLITE_BUSY` error, confirming this statement's
  shape (correlated subquery in `WHERE`, `RETURNING` only `media`'s own column)
  does not reproduce the failure recorded at
  `storage/src/sqlite/sessions.rs:19`. If it does, the SQLite dialect takes the
  two-statement `BEGIN IMMEDIATE` fallback and this criterion is met by that
  form instead — observable behaviour is identical either way.

**Delete guard, end to end (e2e)**

- A18. Upload media, create a post embedding it, attempt delete → refused, with
  the referencing post reported. Force delete → succeeds.
- A19. The guard reports the post when the embedded URL uses the **raw
  filename** spelling, and when it uses the **AtomPub member URL**. These are
  the issue's two headline symptoms; A2/A3 prove the parser, A19 proves the
  behaviour the issue actually asks for — that such a post is _reported as
  referencing_ the media.

**Housekeeping**

- A20. `post_media` is added to the backup golden list
  (`storage/src/backup.rs:699-724`) and
  `backup_covers_every_table_or_deliberately_excludes_it` (`:681`) passes —
  including its two count assertions, the schema comment at `:730` (21 → 22
  backed-up tables) and `live_count` at `:747` (23 → 24). Not
  `backup_table_set_drops_internal_and_denylisted_and_sorts` (`:653`), which
  drives a hardcoded list that a migration cannot move and so cannot detect this
  change.
- A21. The `server-fn-coverage` gate is green after regenerating
  `docs/coverage/server-fns.json` with the new e2e's test names. `media::delete`
  is already covered there (`:369`) with no allowlist entry (retired in #720),
  so no allowlist change is expected; the seed fixture changes only if the new
  e2e needs data the existing fixtures don't provide.

## Follow-up issues (filed as the plan's first task)

1. **[#743](https://github.com/jaunder-org/jaunder/issues/743) — media: allow
   `<video>`/`<audio>` embeds in post bodies.** The sanitiser's allowlist is too
   restrictive for what authors should be able to do. This is a distinct
   user-facing capability with its own security decision — the `SANITIZER`
   comment calls widening exactly that — and it will need `<source>` (format
   alternatives) and `<track>` (WebVTT captions) alongside. Two notes for
   whoever takes it: `ammonia-4.1.4/src/lib.rs:2531` treats `src` as a URL
   attribute on _any_ element and `video[poster]` explicitly, so scheme
   filtering comes for free; `srcset` is **not** in that list, so allowing it
   would admit an unfiltered URL attribute. D3's inverted coupling test forces
   both to be classified before they can land.
2. **[#744](https://github.com/jaunder-org/jaunder/issues/744) — media: match
   absolute media URLs against the configured site host** rather than ignoring
   the host, per D4.

## Related

#675 / ADR-0080 (introduced the encoding whose spelling variance surfaced this),
#708 (the length bound), #720 (blocks this, merged), and the orphaned-file
reclamation issue this unblocks.
