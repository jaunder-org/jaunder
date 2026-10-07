# Org verse and quote rendering (#1697)

## Outcome

Org Post bodies preserve verse layout and quote semantics as Emacs Org HTML
export does. Existing current Posts receive the corrected derived rendering
through an offline upgrade rebuild, not an authored-content edit.

## Load-bearing decisions

- Verse is formatted prose, not literal code: preserve authored line breaks,
  blank lines, and relative indentation while retaining inline Org formatting
  and links. Match Emacs's paragraph-with-breaks semantics rather than `<pre>`.
- Quote blocks render as semantic `<blockquote>` elements with normal Org
  paragraph and inline formatting inside them.
- Lowercase and uppercase block delimiters have the same behavior. Ordinary
  paragraphs, source/example blocks, and unrelated block types retain their
  existing semantics; shortcode-looking text inside verse or quote is not an
  executable Post Shortcode.
- Rendering remains host-owned, and parser output crosses the existing
  sanitization boundary. No author-controlled active markup becomes trusted
  ([ADR-0079](../../adr/0079-rendered-html-sanitization.md),
  [ADR-0202](../../adr/0202-bounded-post-shortcode-embeds.md)).
- Reuse the existing `rebuild_rendered_posts` offline queue operation, enqueued
  by matching SQLite and PostgreSQL upgrade migrations. Existing databases
  receive this request even when they completed all earlier rebuild requests.
- Retain the rebuild contract: refresh changed current derived Post bodies
  (including Deleted Posts) and existing dependent projections, preserve native
  source, semantic edit times, and historical Post Revisions, and invalidate
  affected public Syndication Feeds. Queue consumption stays atomic and
  retryable under the existing offline locking policy
  ([offline migration decision](../../adr/drafts/offline-code-migration-queue.md)).

## Acceptance

- Host rendering tests demonstrate verse paragraph/`<br>` structure with no
  `<pre>`, a blank line, at least two relative indentation depths, inline
  emphasis, and links after sanitization; block markers are not visible. A
  focused regression must fail on the reported collapsed-verse behavior before
  the fix. Multi-paragraph quote tests prove semantic blockquotes, retained
  paragraph boundaries, and preserved inline content.
- Regression cases cover uppercase/lowercase delimiters, unchanged ordinary
  prose and literal blocks, escaped unsafe content, and inert nested shortcode
  text. Assert semantic output rather than incidental exporter wrappers.
- Backend-parametric upgrade tests start with stale stored Org rendering and
  completed earlier rebuilds. Applying the new migration and queue drain
  corrects current rendering while preserving authored source, edit times, and
  historical revisions and AtomPub Member content ETags. Changed bodies retain
  correct Media references and invalidate/enqueue affected public Syndication
  Feeds; unchanged rows create no duplicate event. A subsequent open does not
  repeat completed work. Existing focused rebuild-contract tests may provide
  supporting proof where the operation itself is unchanged.
- Browser proof shows a published Org fixture containing verse and quote on
  Local and its public permalink. Preserve line/blank-line/indentation layout
  and inline styling at narrow (390 px) and wide (1280 px) viewports.
- Comparable before/after screenshots of that fixture's public permalink, using
  the same data, built-in theme, signed-out state, and both viewport widths, are
  surfaced together for visual review.

## Boundaries

No general Org-export parity project, parser replacement, new shortcode syntax,
Theme Package contract change, or historical-revision rewrite. No production
access or live rebuild is authorized here: the upgrade supplies the rebuild. A
short implementation outline is required because paired migrations and stored
projection correctness cross the storage boundary.
