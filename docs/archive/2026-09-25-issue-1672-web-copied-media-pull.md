# #1672 — Localize web-copied Media URLs on explicit Emacs pull

## Outcome

A User who copies a Media URL from the web Post composer into an Org, Markdown,
or HTML Post can explicitly pull that Post into the Emacs Protocol Client and
get a verified Local Media Copy and a working relative local link. This works
for the canonical root-relative URL the web UI actually copies, as it already
does for an absolute same-origin URL.

## Load-bearing decisions

- The configured Jaunder origin is the only authority for resolving a
  root-relative public Media URL. Accept the canonical `/media/upload` and
  `/media/cached` routes only; do not treat protocol-relative URLs, arbitrary
  relative paths, external hosts, or a URL containing a query as eligible.
  Query-bearing URLs stay unchanged and cause no Media fetch.
- Root-relative and absolute same-origin candidates share ADR-0160's strict
  canonical route, lowercase hash, canonical encoded filename, and decoded safe
  local leaf validation. A malformed canonical path shape on an authoritative
  Media route (without a query) must fail closed rather than silently remain a
  remote link.
- Only native-format parser-authorized link/image destinations are candidates.
  Leave plain text, code, unrelated relative URLs, external media, unsupported
  syntax, and surrounding source bytes unchanged. Retain the original fragment,
  where one is supported, and preserve existing absolute same-origin behavior.
- An eligible candidate uses ADR-0160's existing trust chain: an authenticated
  Member's canonical instance UUID agrees with the anonymous direct Media
  response's UUID; no redirects or App Password on the Media request; the strong
  SHA-256 ETag, URL hash, and downloaded bytes agree. Reuse an existing Local
  Media Copy only after verifying its bytes. Store durable copies at
  `local-media/<sha256>/<decoded-filename>` and rewrite only eligible
  destinations to format-appropriate relative local links.
- Install the Post only after all its Media is verified, retaining already
  installed verified copies on late failure. A selected matched `server-ahead`
  pull retains ADR-0200's local/remote revalidation, collision, and
  visiting-buffer protections before replacement.
- `jaunder-reconcile` and its refresh remain inventory/report operations; no
  automatic Media download, Post installation, or publishing is added. The User
  must explicitly select a pull. AtomPub continues to return the stored native
  Post source unchanged.

## Acceptance

- A live web Copy media URL → AtomPub native source → selected Emacs pull
  scenario demonstrates that the root-relative Media destination is localized,
  its verified bytes exist under `local-media/`, and the server Post's native
  source still contains the copied URL.
- Focused Org, Markdown, and HTML tests demonstrate localization of canonical
  root-relative `/media/upload` and `/media/cached` links, continued absolute
  same-origin localization, fragment preservation where supported, and exact
  preservation of unrelated source and parser-excluded syntax.
- Focused rejection tests show that protocol-relative, query-bearing,
  external-host, and unrelated relative URLs remain unchanged with no Media
  fetch, while malformed canonical-route path shapes (without a query) fail
  closed; bad instance identity, redirect, ETag, or bytes cannot yield an
  installed Post or an unverified Local Media Copy.
- Ordinary unselected reconciliation and report refresh issue no Media GET,
  create no Local Media Copy, and neither install nor replace a Post.
- A failing Media fetch leaves a server-only Post absent, and a selected
  `server-ahead` replacement cannot bypass ADR-0200's revalidation or alter the
  local Post on failure. Retried pulls can safely reuse previously verified
  Local Media Copies.

## Boundaries

No reconciliation-side downloading, automatic synchronization, new web/server
endpoint, Media upload-policy change, broad author-URL fetching, parser
replacement, or change to the server's stored Post source. Existing local Media
Copies remain durable managed content, not an evictable cache.
