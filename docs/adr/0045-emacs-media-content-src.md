# ADR-0045: Emacs client harvests media URLs from the response `<content src>`

- Status: accepted
- Date: 2026-07-02
- Issue: [#161](https://github.com/jaunder-org/jaunder/issues/161)

## Context

Publishing an org post (C3, #161) uploads its local images to
`POST /atompub/{user}/media` and rewrites the links in the sent body to the
server's content-addressed URL. The server returns a media-link `<entry>` whose
`<content src>` is the network-resolvable binary URL
(`{base}/media/upload/{sha0..2}/{sha2..4}/{sha}/{filename}`), plus a `Location`
header holding the _edit_ URL. The client needs the binary URL.

## Decision

The client **harvests `<content src>` from the response entry XML** — it does
not use the `Location` header (that is the edit URL) and does not reconstruct
the path from the sha client-side. The server is authoritative about URL layout.

To parse the entry, introduce a shared primitive `jaunder--atom-entry-fields`
(entry XML → alist, on `libxml`/`dom`), **pulled forward from C4 (#162) into
C3**. C3 consumes its `content-src`/`content-type`; C4 and Unit D extend the
field set (slug, published, ETag).

## Consequences

- No second copy of the server's URL scheme in elisp to drift (the epic spec's
  `/media/{sha}/{filename}` shape was already inexact).
- C4/Unit-D reuse one entry parser instead of a throwaway.
- Requires a libxml2-enabled Emacs for `libxml-parse-xml-region`.
