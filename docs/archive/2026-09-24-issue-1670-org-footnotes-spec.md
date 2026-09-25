# Org footnotes render in published Posts (#1670)

## Outcome

Published Org Posts render footnote references and definitions as navigable
notes rather than exposing raw `[fn:…]` text or losing note content. The
production case at `/~mdorman/2013/01/03/visual-versus-logical-lines` renders
its two referenced notes, including the Org link and wrapped text in the first
definition. Authored Org source remains unchanged.

## Load-bearing decisions

- Fix HTML export in the maintained `jaunder-org/orgize` fork, not with regex
  replacement of Jaunder's Org source or a competing footnote parser. Orgize's
  parsed footnote reference/definition events are the authority for eligible
  syntax; code, verbatim, and other literal contexts remain literal.
- Support ordinary Org named references and definitions, inline definitions, and
  repeated references, not just the production document's `[fn:1]` and `[fn:2]`
  syntax. Number by first reference appearance, render a semantic notes section
  with links from each reference to its definition and a usable backlink for
  each reference occurrence. Preserve inline Org formatting and links in note
  contents.
- The stable Post ID owns the fragment namespace: pass it to Org body export on
  create, update, and offline rebuild. Initial creation must obtain the ID
  before producing persisted rendered HTML; a source hash, footnote label,
  display number, or random render-time token cannot stand in for Post identity.
  Fragment targets must be safe and unique across Posts on the same page,
  including identical bodies and repeated labels; displayed numbers remain local
  to each Post. Export must not introduce active markup or circumvent Jaunder's
  existing rendered-HTML sanitization boundary.
- An unresolved reference remains visible as literal source text without a
  dangling link or fabricated note. An unreferenced definition is omitted from
  rendered HTML, but remains intact in authored source; no footnote contents are
  silently lost from storage.
- PR #1671 supplies the reviewed `v0.10` fork pin, offline rendered-Post rebuild
  mechanism, and architecture decision for the fork. Start implementation from
  that merged baseline; advance the pinned fork revision consistently across
  Cargo, tools, Nix vendoring/flake lock and source allowance rather than
  creating a second fork mechanism. Review the fork delta against the prior pin.
- Existing stored Post renderings do not change by changing the exporter alone.
  Enqueue #1671's existing `rebuild_rendered_posts` operation with a new SQLx
  migration for each backend, triggering a full rebuild of all current Posts
  (including retained Deleted Posts), not only Org Posts with footnotes. Refresh
  changed Post derivatives on both storage backends without changing authored
  source, semantic edit timestamps, or historical Post Revisions. For each
  changed body replace that Post's Media references with the exact set derived
  from sanitized new HTML (including referenced note contents). For affected
  public Syndication Feeds invalidate cached representations/validators and
  enqueue the existing `feed_events` notification path, atomically with the
  rebuild, so readers and WebSub subscribers can observe the changed projection.
  Do not emit a semantic Post edit or revision.

## Acceptance

- Fork exporter tests show forward and repeated named references, inline
  definitions, wrapped/multiline definitions with Org links and emphasis,
  numbering in first-reference order, working forward/back links, literal
  unresolved references, omitted unreferenced definitions, and literal contexts.
  Ordinary non-footnote Org export remains unchanged.
- Jaunder host rendering tests confirm the fork output survives sanitization as
  safe, navigable footnotes; two Posts with identical footnote source have
  distinct targets, and creation, update and rebuild use the same Post-scoped
  identity.
- An end-to-end public Org Post test demonstrates references, navigable note
  content and backlinks after publishing; the supplied production source's body
  shape is covered by a regression fixture without copying private Jaunder
  metadata into the test.
- On both SQLite and PostgreSQL, an existing public Org Post whose note includes
  a Media URL gains exactly the sanitized HTML-derived Media-reference set after
  rebuild; its affected Syndication Feed cache and conditional validator are
  invalidated and `feed_events` is queued for eventual WebSub delivery. Source,
  revisions and semantic edit times are preserved. An injected failure rolls
  back HTML, Media references, feed effects and queue deletion together; retry
  completes exactly once under the #1671 offline contract.
- Provide a comparable before/after browser screenshot pair for a public Org
  Post with referenced footnotes at the same viewport, theme, and data state,
  captured before presentation mutation and after the final rendering change.

## Boundaries

- Do not change Org source normalization, AtomPub native-source delivery,
  Markdown/HTML rendering, or the Emacs Protocol Client. Do not add a
  Jaunder-side second Org parser.
- Do not land against the crates.io `orgize` baseline or duplicate merged PR
  #1671's fork/bootstrap migration work. Build on its pinned revision and
  offline queue rather than reimplementing them.
- Do not rewrite historical Post Revisions, silently re-save Posts, or change
  the User's authored footnote labels in stored source.
