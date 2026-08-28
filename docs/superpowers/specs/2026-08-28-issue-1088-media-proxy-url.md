# Issue #1088 — type media proxy request URLs

## Outcome

The media proxy query boundary accepts only canonical absolute HTTP(S) media
source URLs. Invalid proxy URL text is rejected during Axum query extraction
before redirect or deferred cache behavior can run.

## Load-bearing decisions

- `ProxyParams.url` uses the existing `common::tagged_url::MediaSourceUrl`, not
  a new proxy-specific type and not a weightless string wrapper. This keeps the
  inbound request contract coherent with `MediaRecord.source_url` under
  ADR-0063.
- The stable invariant is exactly the existing tagged-URL contract: trim input,
  parse with `url::Url`, require an absolute `http` or `https` URL with a host,
  and retain the canonical serialization established by ADR-0073.
- The temporary redirect `Location` uses the typed URL's canonical form. Case,
  default-port, percent-encoding, and root-path normalization are therefore
  observable boundary behavior rather than preservation of raw query spelling.
- Extractor order remains authentication first, then typed query extraction.
  Unauthenticated requests remain `401 Unauthorized` regardless of malformed
  query contents; an authenticated malformed or disallowed URL is an Axum query
  rejection before the handler runs.
- The existing authenticated `user_id` equality check and its `401` response
  remain unchanged.
- This issue adds no host/address security policy beyond `MediaSourceUrl`.
  Userinfo, explicit ports, path, query, fragment, localhost/private hosts, DNS,
  socket identity, redirect targets, fetching, and cache policy remain governed
  by existing behavior or deferred work.
- This is an application of ADR-0063, ADR-0073, and the existing stored-media
  contract. It creates no new architectural decision or glossary concept.

## Acceptance

- An authenticated canonical or normalizable absolute HTTP(S) URL produces the
  existing temporary redirect with its canonical URL in `Location`.
- Authenticated malformed, relative, empty-host, and non-HTTP(S) URL inputs are
  rejected with `400 Bad Request` by query extraction.
- An unauthenticated malformed URL still returns `401 Unauthorized`, proving
  authentication precedence is unchanged.
- A valid URL paired with a different authenticated `user_id` still returns
  `401 Unauthorized`.
- Production proxy request flow carries `MediaSourceUrl`; no raw proxy URL
  string field or compatibility path remains.
- Focused Playwright coverage exercises the authenticated proxy endpoint's
  canonical redirect and invalid-URL rejection without following the external
  redirect.
- The resulting code or decision record references issue #697 and ADR-0063;
  focused media-handler, Playwright, and repository static checks pass.

## Boundaries

- No remote media fetch, cache write/read, response-body proxying, or redirect
  following.
- No SSRF, DNS-rebinding, private-address, port, userinfo, query, or fragment
  policy beyond the existing `MediaSourceUrl` validator.
- No change to `MediaRecord.source_url`, media storage schema, public media
  serve paths, authentication, or query `user_id` semantics.
