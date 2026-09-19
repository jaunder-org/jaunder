# ADR-0201: Emacs Local Post Links Round-Trip Through Canonical URLs

- Status: accepted
- Date: 2026-09-19
- Issue: [#1586](https://github.com/jaunder-org/jaunder/issues/1586)

## Context

The Emacs Protocol Client currently treats every body-level Org `file` link as
media. Consequently, a valid relative link such as `./another-post.org` is
uploaded as `application/octet-stream` and replaced only in the sent body by a
media URL. Authors instead need relative links between local Post files to stay
navigable in Org while the published Post uses the target's public permalink.

Local filenames are not Post identity, and Jaunder's public URL layout is owned
by the server. Collection reconciliation already joins local and remote Posts by
Post ID, while a Member advertises its public permalink through
`rel="alternate"` only when the server has one to advertise. Pull may also run
with only part of the Collection present locally, so converting every public URL
back to a guessed filename would make local state and pull order accidental
identity authorities.

[ADR-0024](0024-server-side-org-canonicalization.md) requires the authored local
representation and canonical served representation to differ without corrupting
each other. [ADR-0045](0045-emacs-media-content-src.md) establishes the related
rule that server-assigned URLs are harvested rather than rebuilt.
[ADR-0047](0047-emacs-publish-orchestration.md) requires validation before
server mutation, and [ADR-0200](0200-revalidated-matched-post-pull.md) protects
matched pull replacement with snapshot revalidation.

## Decision

A relative body-level Org `file` link whose filesystem path ends in `.org` is a
**Local Post Link candidate** and is claimed before media. It becomes a **Local
Post Link** only when it has no search target, query, or fragment and passes the
remaining checks. An invalid candidate never falls through to media upload. Its
path resolves normally from the linking file; the client never searches for a
matching basename. The exact target must be a regular file inside the same
configured Jaunder root, and its Post ID, slug metadata, and `<slug>.org`
filename must agree with one Collection Member.

Before server mutation, publish resolves every candidate and requires exactly
one direct-child Atom-namespace link whose `rel` is exactly `alternate`. Its
`href` must be an absolute HTTP(S) URL on the active Jaunder origin without user
information, query, or fragment; duplicate matching links are invalid even when
their values agree. Missing, escaped, ambiguous, stale, draft, or otherwise
invalid targets produce a visible warning and abort the publish. The client
never normalizes or reconstructs a permalink from a filename, slug, Post ID,
username, or route convention. It substitutes the exact harvested `href` only in
the sent body; the authored body and link bytes remain unchanged while ordinary
publish metadata write-back and rename behavior continue.

Pull performs the inverse only with complete proof and only for body-level Org
HTTP(S) link destinations. A destination is rewritten to `./<slug>.org` when its
string exactly equals, without URL normalization, one Member's harvested
canonical `href` and an existing local file in the same root has that Member's
exact Post ID, slug metadata, and filename. Only the destination span changes;
descriptions, surrounding bytes, non-link text, code, and metadata remain
unchanged. Without that unique agreement, pull preserves the canonical URL. It
never searches for or pulls a missing target as a side effect, and localization
remains inside the existing staged server-only and revalidated matched-pull
safety boundaries.

Absolute paths, `attachment:` links, cross-root links, and links carrying Org
search targets, queries, or fragments are outside this contract. Markdown and
HTML gain no corresponding local syntax in this decision.

## Consequences

Authors keep ordinary navigable relative links in local Org while Jaunder stores
and serves authoritative public URLs. Relative `.org` files no longer fall
through to generic media upload, so an invalid intended Post link fails visibly
rather than becoming an opaque attachment.

The server remains the sole permalink authority, and local filename changes
cannot silently invent a new public identity. Pull can restore local navigation
for fully reconciled targets without treating partial local inventory as proof;
a canonical URL remains valid source when its target is absent or mismatched.

Publish gains read-only Member resolution before mutation, and pull gains an
identity-aware Post-link localization pass. Both directions require tests for
ambiguous or stale metadata, partial inventories, path containment, and
composition with existing media and pull-safety rules. Unsupported search or
fragment semantics remain explicit rather than being mapped to unstable HTML
anchors.
