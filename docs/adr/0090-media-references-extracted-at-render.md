# ADR-0090: A post's media references are derived from its sanitized HTML, never supplied

- Status: accepted
- Date: 2026-07-30
- Issue: [#711](https://github.com/jaunder-org/jaunder/issues/711)

## Context

Until now, "which posts reference this media item?" was answered by
substring-searching post bodies for the exact derived serve URL. That is wrong
three ways: a post spelling the same file differently (raw filename rather than
the percent-encoded one, or the AtomPub member URL, which shares none of the
serve URL's prefix) does not match; the scan is capped at 1000 published posts
and 1000 drafts, so a reference past the window is invisible; and it cannot
answer the reclamation question ("is this on-disk entry referenced by any
post?") without reading every corpus, per file.

Three forces shape the fix.

**Where the truth is.** A post's source body is not what determines what it
points at — the _sanitized rendered HTML_ is. Sanitisation strips elements that
can never render, and Markdown and Org both pass embedded raw HTML through
untouched, so a raw `<img>` in a Markdown body is as real a reference as a
Markdown image.

**Publication was misfiled as an edit.** `common::render::render` is the single
rendering chokepoint, with three production callers, all in
`storage::post_service`. It is tempting to conclude that recording references
there covers every write. It did not: publishing a draft
(`web/src/posts/api.rs`) wrote a full post _update_ built from the _stored_
record, deliberately not re-rendering, because publication must not alter
content. Any design in which callers _supply_ a reference set gives that path
nothing to supply — so the busiest write in the product would silently erase the
rows it should preserve. The path itself is the defect: publication changes one
timestamp, and routing it through the machinery for rewriting a post's content
is what created the hazard.

**What a URL actually names.** A bare content hash is too coarse: identical
bytes stored under two filenames are two distinct (hard-linked) directory
entries, so a hash-only relation could neither decline the right deletion nor
identify the right orphan.

## Decision

**A post's media reference set is _derived_ from its sanitized HTML by a pure
function, and is never supplied by a caller.**

1. **The reference set is unforgeable.** The render output type carries the HTML
   and the reference set together, with **both fields private** and rendering as
   its _only_ constructor. `CreatePostInput`/`UpdatePostInput` carry that type
   in place of a bare HTML field. A value whose reference set disagrees with its
   HTML is therefore unrepresentable — not merely discouraged. Rows are written
   in the post's own transaction.

   Both fields, because the disagreement is reachable from either side: a public
   reference set would let a caller supply a wrong one, and a public HTML field
   would let a caller swap the HTML out from under the set derived from the
   original. Reading is by accessor, and taking the HTML _out_ consumes the pair
   — so the set can never outlive the HTML it describes.

   This is deliberately stronger than passing HTML and references as two fields:
   that form makes the _correct_ thing possible but the _empty_ thing equally
   easy, which is the drift-by-omission this decision exists to eliminate.

2. **Publication gets its own storage operation** that sets the publication
   timestamp and touches nothing else — no body, no rendered HTML, no slug, no
   audiences, no media rows. This is what lets rendering be the sole
   constructor: with publication out of the update path, every remaining writer
   of post content renders.

   The considered alternative — a second constructor that adopts stored HTML and
   re-derives its references, letting publication keep using the update path —
   was rejected. It re-parses the whole stored body on the product's busiest
   write to recompute rows that are already correct, then deletes and reinserts
   them unchanged, and it exists only to serve a call site that should not be
   there. A door added to accommodate a misuse makes the misuse permanent.

3. **A reference is a URL, in a position where the post points a reader at it,
   that _names_ a stored media entry.** Naming, not loading: the AtomPub member
   URL is counted even though it serves an Atom document rather than bytes,
   because both consumers ask a naming question — the delete guard asks whether
   removing a record would contradict something a post says, and reclamation
   asks whether anything names an on-disk entry.

4. **The sanitiser's allowlist and the extractor's surface are coupled by a
   test, and the extractor's surface is data.** Extraction reads the sanitized
   output, so what can be referenced is bounded by what survives sanitisation.
   Those two lists living apart is the same drift-by-omission this ADR exists to
   close, one level up: widen the allowlist, forget the extractor, and the
   relation acquires a silent blind spot.

   The coupling is enforced by requiring every `(element, attribute)` pair the
   sanitiser permits to appear in either the extractor's table or an explicit
   inert list — an _inverted_ assertion, so a newly permitted pair fails until a
   human classifies it. Testing "the URL-bearing ones are covered" instead would
   need a hand-written URL-attribute predicate (ammonia's is private), and such
   a predicate would not recognise a multi-URL attribute like `srcset` —
   precisely the case most likely to be got wrong.

   Both lists are **declarative tables that the extraction walk is driven by**,
   never tag names embedded in code, so admitting `<video>`/`<audio>` is a data
   edit rather than a rewrite. A test drives the walk with a synthetic pair to
   keep that true.

   Because this obligation binds whoever edits the _sanitiser_, it is documented
   at the sanitiser's definition — not only in the tables and the test — so it
   is visible at the moment someone is widening the allowlist rather than
   discoverable afterwards.

5. **References are keyed on the `(source, sha256, filename)` triple** — what a
   URL names, and what names one on-disk entry.

6. **Matching is host-blind.** Any URL whose _path_ parses as a known media
   layout is a reference, whatever scheme, host or port it claims. Rendering is
   pure and has no access to configuration; threading a site host through it
   would change every caller, make rendering config-dependent, and — because the
   rendered HTML is stored — let a later hostname change silently invalidate
   what was already extracted.

7. **Filenames are canonicalised through the read path's existing door**
   (percent-decode, then `ProfferedFilename`), on the write path, once.
   Normalising at extraction rather than at comparison is the point: a transform
   at a comparison point is precisely the bug class this decision closes.

8. **The deletion guard is one conditional statement, so the policy lives in
   storage.** Asking "is it referenced?" and then deleting leaves a window in
   which a post can start referencing the media between the two — and closing
   that window with a transaction is surprisingly expensive on Postgres, where a
   concurrent insert of a _new_ reference row conflicts with nothing the check
   read, so only SERIALIZABLE isolation or locks taken by the post write path
   would catch it.

   A `DELETE … WHERE NOT EXISTS (…) RETURNING` makes the question and the answer
   the same statement, which is atomic in both engines: no transaction, no
   locking, no isolation levels. The cost is that "refuse unless forced" becomes
   a storage rule rather than a handler rule — the correct home for it anyway,
   since it is a data-integrity invariant over storage's own tables, and it
   leaves the handler doing nothing but reporting.

   This follows the same move ADR-0021 made for feed-event claiming, where a
   single `UPDATE … RETURNING` replaced a transaction.

## Consequences

- The relation is authoritative only for posts written or edited after it lands.
  This is acceptable because there are no production users; a deployment with
  existing content would need a backfill pass, which the pure extractor below
  makes a loop over stored `rendered_html` rather than a re-render of every
  post.
- Deletion guards and future orphan reclamation become indexed lookups: no scan,
  no truncation, and no viewer-visibility filtering — a post hidden from a
  viewer still references its media.
- A pure `HTML → references` function becomes load-bearing infrastructure, not
  an implementation detail: it is what the coupling test enumerates against,
  what makes a backfill cheap, and what reclamation will reuse. It is
  deliberately _not_ reachable as a way to construct the render output type.
- Publication becomes a narrower, cheaper operation than it was: it no longer
  reads and rewrites a post's audiences purely to avoid clobbering them.
- Host-blind matching errs toward "referenced" in both consumers: a delete that
  could have proceeded is refused (which `force` covers), and an orphan is left
  unreclaimed rather than a live file deleted. Host-aware matching is left as
  follow-up work.
- Adding _any_ element or attribute to the sanitiser now carries an obligation
  to classify it as media-bearing or inert, enforced by test rather than by
  review attention. This is a deliberate tax on widening the allowlist, and it
  is paid in the same edit rather than in a later bug.
- Attributes naming more than one URL (`srcset` and its kin) do not fit the
  extractor's table and cannot be admitted without widening its shape. The
  coupling test surfaces that as a failure rather than letting a single-URL
  parse quietly mangle them.
- Reading rendered HTML rather than the source body narrows what counts: a media
  URL that survives only as literal text (inside a fenced code block, say) no
  longer registers. That follows from the principle — text points nobody at
  anything.
- Storing the relation without a `user_id` column keeps "referenced by anyone?"
  answerable without a schema change, even though the current delete guard
  scopes to the deleting user's own posts.
- The list of referencing posts shown to the user stays a separate, advisory
  query. It populates a message, not a decision, so it need not share the
  guard's atomicity — and keeping it out of the conditional statement avoids a
  Postgres-only data-modifying CTE that SQLite cannot express, which would mean
  two divergent implementations of one operation.
- Folding the check into the delete makes "nothing was deleted" ambiguous
  between "not found" and "refused", so the failure path pays one classifying
  query to keep those distinct. That query is advisory and cannot reopen the
  race, because the decision has already been made.
- Rendering now costs one extra HTML tokenisation per post write. Post writes
  are not a hot path, and this buys the guarantee that HTML and references
  cannot disagree.
