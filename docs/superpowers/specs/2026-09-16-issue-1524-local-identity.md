# Operator-configurable Local identity

Issue: #1524

## Outcome

Operators can configure the title and optional Site Tagline that identify Local
through the existing Site Settings interface. Projected first paint, reactive
Local rendering, HTML metadata, and applicable Syndication Feeds present the
same safely rendered identity.

## Load-bearing decisions

- `site.title` remains the required title and retains its existing `Jaunder`
  default when no valid configured value exists.
- Add `site.tagline` as an optional Site Tagline in the closed
  site-configuration registry.
- A Site Tagline is single-line plain text. Parsing trims its edges, treats
  blank input as absent, preserves casing and interior whitespace, and accepts
  at most 280 Unicode scalar values after trimming. Carriage return, line feed,
  next line (U+0085), line separator (U+2028), and paragraph separator (U+2029)
  are rejected anywhere in the value.
- An invalid persisted `site.tagline` value reads as absent without repairing
  the row, matching the existing defensive identity fallback; storage failures
  still propagate as errors.
- The operator edits title, tagline, and canonical base URL in the existing Site
  Settings card. One save validates and commits the three values atomically.
- Local renders the configured title as its prominent masthead heading. It
  renders a tagline element only when a Site Tagline is present.
- The shared Local masthead renderer remains the sole markup source for
  projected and reactive output, preserving byte coincidence and avoiding
  first-paint layout shift.
- Local HTML metadata uses the configured title and optional Site Tagline for
  the document title, standard description, and Open Graph title and
  description.
- The Site Tagline supplies RSS channel `description`, Atom feed `subtitle`, and
  JSON Feed `description` for site-wide and site-tag Syndication Feeds.
- User and User-tag Syndication Feeds do not inherit the Site Tagline because it
  does not describe an individual User's publication.
- All title and tagline rendering uses text semantics; operator input never
  becomes markup.
- Changes become visible on the next server-fetched Local response or in-app
  navigation. Existing open pages need not update live, and a cached projected
  document may retain the prior identity for its existing five-minute public
  freshness window before revalidation.
- The existing public-theme Style Contract remains valid. Title and optional
  tagline continue through its stable semantic masthead hooks rather than adding
  theme-specific markup.

## Acceptance

- Site Settings loads the persisted title and tagline in operator-editable
  controls alongside the canonical base URL.
- An operator can set, change, and clear the Site Tagline; re-entering Site
  Settings shows the persisted result.
- Invalid title, tagline, or base URL input prevents the aggregate save before a
  write begins. A rollback-confirmed operation failure leaves all three prior
  values intact; a commit-indeterminate outcome revalidates and tells the
  operator to reload rather than claiming either success or rollback.
- With no configured title or tagline, Local displays `Jaunder` and emits no
  tagline element or description content derived from one.
- With both configured, projected `/` HTML and the mounted reactive Local view
  display identical escaped title and tagline text without layout shift.
- Local document and Open Graph metadata carry the configured title and tagline.
- Site-wide and site-tag RSS, Atom, and JSON representations carry the Site
  Tagline in their native feed-description field; User and User-tag feeds omit
  it.
- Host rendering and protocol tests cover configured, absent, invalid persisted,
  boundary-length, overlong, whitespace-only, each prohibited line separator,
  interior whitespace, Unicode, and markup-looking tagline values.
- Storage and aggregate mutation behavior is covered on SQLite and PostgreSQL.
- End-to-end coverage proves the operator UI round-trip and the resulting Local
  presentation while preserving existing Local and theme behavior.

## Boundaries

- Do not reintroduce the promotional hero removed by #1512.
- Do not add per-User or per-Post taglines, User biographies, or Post summaries.
- Do not display the Site Tagline on Home or use it for User/User-tag feed
  descriptions.
- Do not add live cross-tab synchronization.
- Do not change public-theme packaging, executable capabilities, or unrelated
  Style Contract concepts.
