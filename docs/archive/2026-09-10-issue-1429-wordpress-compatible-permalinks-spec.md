# Issue #1429 — WordPress-compatible permalinks

## Outcome

Jaunder accepts an inbound WordPress-style post permalink at `/YYYY/MM/DD/slug`
and resolves it without requiring the User segment. When that path identifies
one active Post visible to an anonymous viewer, the server responds with a
same-origin HTTP 302 redirect to that Post's canonical
`/~username/YYYY/MM/DD/slug` permalink. The redirect preserves the received
query string byte-for-byte after `?`.

Canonical URL ownership remains unchanged: Jaunder emits canonical links only,
and client-side routing recognizes only the `~username` form. The compatibility
form is an inbound server alias, not a second canonical URL or CSR route.

## Load-bearing decisions

- The alias is inbound-only `GET /YYYY/MM/DD/slug`; it does not extend other
  methods, URL shapes, or resource types.
- Lookup searches active Posts visible to `ViewerIdentity::Anonymous` across all
  Users, without a login, ownership, or preconfigured single-user assumption.
- Exactly one matching active Post is required before redirecting. The redirect
  target is constructed from that Post's canonical User and Post permalink
  components, rather than echoing an inferred alias as a canonical address.
- The redirect is same-origin, uses HTTP 302, preserves any original query
  string unchanged, and includes `Cache-Control: no-store`.
- A zero-match, multiple-match, or malformed alias request is a public miss:
  return the existing public SPA shell miss with `Cache-Control: no-store`, not
  a redirect, detail page, error body, or disclosure of lookup ambiguity.
- Every supported storage backend must provide the same active-Post lookup and
  observable alias result; backend choice must not change visibility, matching,
  redirect, or miss semantics.
- The design is recorded as proposed in
  `docs/adr/0189-wordpress-compatible-permalink-alias.md`.

## Acceptance

- An anonymous request for `/2026/07/12/sandbox-post-14` that has exactly one
  active Post match visible to `ViewerIdentity::Anonymous` receives a
  same-origin 302 whose location is that Post's canonical
  `/~username/2026/07/12/sandbox-post-14` permalink.
- The redirect location retains an input query string exactly, including its
  ordering, repeated keys, and empty values; a request without a query has no
  added query delimiter.
- The successful alias response carries `Cache-Control: no-store`.
- A syntactically malformed compatibility path, a path with no active Post
  match, and a path with more than one active Post match each receive the
  existing no-store public SPA shell miss, never a redirect or match-specific
  information.
- The alias can resolve an active Post owned by any User while unauthenticated.
  Inactive Posts and active Posts hidden from anonymous viewers produce the same
  no-store public SPA shell miss.
- Rendered Jaunder links remain canonical `~username` permalinks, and CSR
  navigation continues to accept only that canonical form.
- The observable behavior is identical for every supported storage backend.

## Boundaries

- This does not change canonical permalink syntax, emitted link generation, or
  CSR route recognition.
- This does not add a public listing, search endpoint, ambiguity resolution UI,
  username inference setting, or a second URL form for non-Post resources.
- This does not turn missing, malformed, or ambiguous aliases into a 404,
  redirect to a guessed User, or a response that exposes candidate Posts.
- This does not alter authenticated access rules beyond allowing the defined
  anonymous lookup of active Posts for this inbound compatibility alias.
