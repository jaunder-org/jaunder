# ADR-0140: The public media route extracts one strict validated address

- Status: accepted
- Date: 2026-08-15
- Issues: [#692](https://github.com/jaunder-org/jaunder/issues/692),
  [#1046](https://github.com/jaunder-org/jaunder/issues/1046)

## Context

The public content-addressed media route is five segments:

`/media/{source}/{p1}/{p2}/{hash}/{filename}`

Three segments already have domain types: `MediaSource`, `ContentHash`, and the
inbound `ProfferedFilename`, which recovers canonical `Filename` bytes after
Axum percent-decodes the URL segment. The two prefix segments are redundant
projections of the hash.

Issue #504 deliberately wrapped the typed segments in `SoftPath<T>`. Parse
misses reached `serve_handler` as `None` and became 404, making malformed
addresses indistinguishable from syntactically valid but absent media. That was
useful for uniform public soft-route behavior, but it carried invalid parse
state into the handler and left the cross-field prefix/hash invariant to an
ordinary function branch.

The policy has changed. For this opaque content-addressed route, a malformed
address is a malformed HTTP request, not a domain lookup miss. Invalid address
state should be unrepresentable after extraction. The projector and Syndication
Feed routes have different navigation/fall-through semantics and are not part of
this decision.

The route also emits `jaunder.media.served{result=ok|not_found|not_modified}`
inside the application. Strict extractor failures occur before the handler and
therefore cannot be counted there without special rejection plumbing. More
importantly, the deployment's front proxy is the authoritative observer of HTTP
request status. A partial application counter is liable to be interpreted as
complete.

[ADR-0084](0084-media-filename-encoded-canonical.md) makes `Filename`'s encoded
spelling canonical and confines `ProfferedFilename` to inbound extractor
positions. That decision remains valid; only its media-serve extractor shape
changes.

## Decision

**The public media route extracts one strict validated address.**

A private extraction type consumes all five route segments and establishes every
address invariant before `serve_handler` runs:

- parse `MediaSource`, `ContentHash`, and `ProfferedFilename`;
- compare both supplied prefix segments with the corresponding leading pairs of
  the validated hash;
- convert `ProfferedFilename` infallibly to canonical `Filename`; and
- retain only `MediaSource`, `ContentHash`, and `Filename`.

Any parse failure or prefix mismatch fails Axum extraction with HTTP 400.
`serve_handler` and every lower helper can receive only a valid media address. A
valid address whose on-disk file is absent continues to return 404; a present
file without a database row retains the existing extension-derived 200 fallback.

This supersedes #504's soft-404 decision **only for the public media route**.
`SoftPath` remains valid for projector and Syndication Feed routes whose miss
behavior intentionally falls through or renders a shell.

The `proffered-filename-position` gate permits `ProfferedFilename` only as an
implementation detail of the validated Axum media-address extractor and the
existing strict AtomPub `Path` extractors. It continues to reject the type in
DTOs, storage, return values, server-function parameters, and ordinary helper
parameters.

**HTTP serve-outcome counts belong to the front proxy.** Remove
`jaunder.media.served` and its `ServeResult` classification rather than carrying
a partial in-app count. Media upload outcome and byte metrics remain:
stored/deduplicated/quota/application semantics are not observable at the proxy.

## Consequences

- Malformed source, hash, filename, or prefix/hash combinations change from
  handler-level 404 to extractor-level 400. Router tests pin this intentionally.
- A valid address with a missing file remains 404; a present file without a
  database row remains 200 with extension-derived content type. Successful
  response bytes, headers, URL layout, and canonical filename spelling do not
  change.
- The handler loses optional parse state and prefix checks. Post-extraction
  invalid media addresses are structurally impossible.
- Tests submit malformed values as URLs through the router; no unit helper
  bypasses extraction with adjacent raw strings.
- [ADR-0084](0084-media-filename-encoded-canonical.md) is amended only in its
  `ProfferedFilename` extractor/gate consequence. Its encoded-canonical
  representation, safe-leaf oracle, intake order, and storage/wire decisions
  stand.
- Projector and Syndication Feed soft routes are unchanged. This is not a
  repository-wide claim that every malformed route must return 400.
- The application no longer exposes `jaunder.media.served`; operators obtain
  HTTP status counts from the front proxy. Upload-domain metrics remain
  available.
