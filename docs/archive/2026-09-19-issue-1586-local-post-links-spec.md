# Local Post links through canonical public URLs (#1586)

## Outcome

An author can link Posts in one configured Jaunder root with ordinary relative
Org links such as `[[./another-post.org][Another Post]]`. The Emacs Protocol
Client keeps that link useful on disk, publishes the target's
server-authoritative public URL, and restores a relative link on pull only when
local and remote identity agree exactly.

## Load-bearing decisions

- A **Local Post Link candidate** is a body-level Org `file` link whose target
  is relative and whose filesystem path ends in `.org`; `[[./post.org]]` and
  `[[file:./post.org]]` are equivalent. Every candidate is claimed before media
  classification and never falls through to generic media upload. It becomes a
  **Local Post Link** only when it has no Org search target, query, or fragment
  and passes every remaining check.
- The path resolves normally from the linking file, never through a basename
  search. Its exact target must be a regular file inside the same configured
  root; symlink or `..` escapes and cross-root targets are invalid. Its
  `JAUNDER_ID`, `JAUNDER_SLUG`, and `<slug>.org` filename must agree with one
  Collection Member, so local drafts and stale or orphan identities are invalid.
- The Member must contain exactly one direct-child Atom-namespace `link` whose
  `rel` is exactly `alternate`. Its `href` must be an absolute HTTP(S) URL on
  the active Jaunder origin without user information, query, or fragment.
  Duplicate links are invalid even when their `href` values agree.
- The harvested `href` string is authoritative. The client neither normalizes it
  nor derives a permalink from a path, slug, Post ID, username, or route. Any
  invalid candidate produces a visible warning and aborts before server mutation
  rather than publishing a likely broken link.
- Publish substitutes the `href` only in the body sent to Jaunder. It preserves
  authored body and link bytes, including descriptions; ordinary successful
  metadata write-back and `<slug>.org` rename behavior remain in force.
- Pull considers only body-level Org HTTP(S) link destinations. It rewrites a
  destination to `./<slug>.org` only when the string exactly equals, without URL
  normalization, one Member's harvested `href` and an existing same-root file
  has that Member's exact Post ID, slug metadata, and filename. Only the
  destination span changes: descriptions, surrounding bytes, non-link text,
  code, and metadata remain unchanged.
- Pull reversal is opportunistic. Without the complete unique proof it preserves
  the canonical URL; it never searches for or pulls a missing target. Post-link
  localization stays within existing staged-install, no-overwrite, identity,
  ETag, local-digest, modified-buffer, and canonical-destination protections.

## Acceptance

- Publishing either supported spelling of a valid Local Post Link sends the
  exact harvested `href`, performs no media upload for the `.org` file, and does
  not alter the authored body or description; normal publish write-back and
  rename still occur.
- Repeated links to one target resolve consistently, and distinct targets use
  their respective server-advertised URLs.
- Every invalid class above warns, leaves the server unmodified, and cannot fall
  through to media upload. Missing, malformed, cross-origin, or duplicate
  `rel="alternate"` links are rejected, including identical duplicates.
- Pull rewrites only an Org body-link destination whose string exactly matches a
  harvested canonical `href` when Member ID and slug, local metadata, filename,
  and root all match uniquely.
- Pull preserves the URL for absent or mismatched proof—including a
  normalization-equivalent but non-identical URL or a target not present
  locally—and leaves descriptions, surrounding bytes, non-link text, code, and
  metadata unchanged.
- Existing media localization, external links, publish retry safety, server-only
  pull, and matched server-ahead pull retain their current behavior and ADR
  invariants.

## Boundaries

- This covers Org Posts in the Emacs Protocol Client, not Markdown or HTML.
- `attachment:` links, absolute file links, cross-root links, and links with Org
  search targets, queries, or fragments are unsupported.
- No permalink API or template is added; Atom `rel="alternate"` remains the
  authority.
- The client does not search for, auto-pull, rename, or repair a missing target.
  Relative links remain local; canonical public URLs remain the wire and stored
  representation.
