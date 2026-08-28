# ADR-0097: Post DTOs are named for the content weight they carry

- Status: accepted
- Date: 2026-08-01
- Issue: [#569](https://github.com/jaunder-org/jaunder/issues/569)

## Context

The posts vertical carries several post-shaped wire types. Before #569 they were
named for their **role in the plumbing** — `PostResponse`, `CreateResult`,
`UpdateResult`, `PublishResult`, and an `args` wire key — or for a vague size
adjective: `DraftSummary` alongside `TimelinePostSummary`.

Neither naming scheme encoded the property that actually separates them, which
is **content weight**:

| Tier              | Carries                   | Example after #569 |
| ----------------- | ------------------------- | ------------------ |
| metadata only     | identity, labels, links   | `UnpublishedPost`  |
| + rendered form   | `RenderedHtml`            | `RenderedPost`     |
| + authored source | `PostBody` + `PostFormat` | `AuthoredPost`     |

The cost of not encoding it was concrete rather than aesthetic.

**Unrelated types read as redundant.** `DraftSummary` was metadata-only while
`TimelinePostSummary` carried full `RenderedHtml` and was the second-heaviest
wire read DTO in the codebase — the same suffix on opposite tiers. A reader
comparing `PostResponse`, `PostRecord`, and `PostView` had to open all three and
diff their fields to learn which was which.

**That misreading produced a wrong proposal.** #747 was filed proposing a merge
of the two `*Summary` types — the weakest candidate in the family (0.31
shared/union) and the only one crossing a content tier. Had it landed, timeline
pages would have shipped `rendered_html` for rows that never render it, or
drafts rows would have carried post bodies. The names invited the error; the
evidence, once gathered, refuted it.

**And genuinely duplicated types went unnoticed.** While the family looked
redundant where it was not, `PostResponse` really was a strict superset of
`TimelinePostSummary` — eleven identical fields — and the code hand-converted
between them field by field, fabricating a `published_at` for drafts along the
way. That duplication had sat unmerged for as long as the false one had looked
mergeable.

## Decision

**Name a post-shaped wire type for the content weight it carries, not for the
transaction that produced it or its size relative to a sibling.**

Three rules follow:

1. **No transaction-role suffixes.** `*Result` and `*Response` describe how a
   value reached the caller, not what it is. One type serves all four post
   mutations (`SavedPost`); the read types are named for their tier.

2. **A merge is viable only _within_ a tier.** Two types in the same tier with a
   high shared/union ratio are candidates. A merge _across_ tiers is a wire
   regression by construction: the union ships the heavier tier's payload to
   consumers of the lighter one. Concretely, unioning `RenderedPost` and
   `AuthoredPost` would put a `PostBody` on all 50 rows of a timeline page
   (`PageSize::default()` is 50).

   The remedy for a cross-tier pair that _looks_ redundant is a **shared core
   plus an extension**, not a union:
   `AuthoredPost { post: RenderedPost, body, format }` gives the timeline the
   core alone and the permalink both, with no unread payload either way.

3. **Structural overlap alone does not justify folding two types together.** The
   discriminator is whether the code _converts between them_. `AuthoredPost` and
   `RenderedPost` were folded because `PostPage` was rebuilding one from the
   other by hand. `SavedPost` and `UnpublishedPost`'s row overlap exactly in all
   four of `SavedPost`'s fields — but nothing anywhere converts between them, so
   `UnpublishedPost` _nests_ a `SavedPost` rather than the two collapsing into
   one type. A flat union there would have put `title`, `summary_label`, and
   `edit_url` on every mutation response, where nothing reads them.

## Consequences

**A reader can place a type without opening it.** `RenderedPost` versus
`AuthoredPost` answers "does this carry the source?" from the name. That is the
property that decides whether a value is cheap to ship in a list.

**Merge proposals now need tier evidence.** "These two look alike" is not
sufficient grounds; the question is which tier each occupies and whether
anything converts between them. This ADR exists largely so the #747 analysis
does not have to be redone — that issue was rewritten to its storage-layer work
once the evidence was in.

**The `SavedPost` ↔ `UnpublishedPost` overlap is deliberate, not an oversight.**
Four fields are declared in two places. A future reader who spots that and files
it as duplication should read rule 3 first: the absence of any conversion
between them is the reason, and it is recorded here so the finding does not
recur.

**This does not reach beyond the posts family.** `common::media::UploadedMedia`,
`web::auth::SessionUser`, and `web::media::MediaDeletion` are vertical-specific
content names; they do not extend this posts content-weight axis.

**Nor does it reach the storage layer.** `storage::PostRecord` and
`storage::helpers::PostRow` are a database record and a `sqlx` row; they are not
wire DTOs and this convention does not govern them.
