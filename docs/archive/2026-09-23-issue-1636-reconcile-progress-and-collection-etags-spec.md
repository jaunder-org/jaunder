# Reconciliation progress and Collection ETags (#1636)

## Outcome

Opening or refreshing the Emacs reconciliation report visibly indicates that it
is fetching and classifying current Posts, rather than appearing frozen. For a
Jaunder server that advertises per-Post validators in its AtomPub Collection, a
report no longer needs a separate HTTP request for every matched Post; the
Collection remains a public, interoperable AtomPub surface.

## Load-bearing decisions

- Retain the paginated AtomPub Collection as the inventory source. Add exactly
  one read-only `<j:etag>` direct child to each Collection Entry, where `j`
  denotes `https://jaunder.org/ns/atompub` (the prefix itself is not
  significant). Its sole content is the exact quoted strong Post ETag sent in
  that Entry's Member `GET` response, with no padding whitespace or attributes;
  incoming `j:etag` values on writes are ignored. Protocol Clients unaware of
  the extension may ignore it. Do not create an Emacs-only endpoint or persist
  ETags in storage.
- Advertise the additive `member-etag` feature token in version `1` of the
  existing Jaunder Service Document `j:extension` capability. The Collection
  Entry itself is the read authority: a Protocol Client may use a valid `j:etag`
  without first fetching the Service Document, and a missing feature token does
  not suppress a valid Entry value.
- Compute the advertised validator from the same Post content and complete
  audience target set used by the Member response and conditional writes. The
  extension is metadata about the Member's mutable representation, **not** the
  ETag of the Collection page or the serialized Entry.
- On a valid Collection validator, use it when classifying a matched local Post.
  If exactly one direct Jaunder-namespace `etag` element with exact strong-ETag
  text is not available, retain the existing per-Member `GET` path and its
  row-local error reporting, including for older servers. Duplicate, malformed,
  weak, whitespace-padded, attributed, nested, or namespace-spoofed elements
  must not become trusted validators.
- A report remains a preview, not authority for a future mutation. Preserve
  existing fresh remote checks, conditional requests, local preflights, explicit
  selection, and conflict/blocked behavior when pushing, pulling, or deleting
  Posts.
- For both initial `jaunder-reconcile` and report refresh (`g`), display a
  truthful indication before synchronous inventory and classification work,
  including the remote fetch. Clear or replace it on success and failure; do not
  claim background execution or silently discard the previous report on refresh
  failure. Keep batch-action progress distinct.
- Record the public wire contract as a proposed ADR and project it into
  `docs/ARCHITECTURE.md` alongside implementation, extending the existing
  Jaunder AtomPub extension convention without replacing Atom serialization.

## Acceptance

- A Collection page, including paginated pages, exposes exactly one direct
  `https://jaunder.org/ns/atompub` `etag` element per Entry, matching the `ETag`
  header of that Post's Member `GET` for identical server state (including
  audience-only changes); version-1 Service Document discovery advertises
  `member-etag`. No Collection page-level ETag is confused with it. Existing
  Atom consumers remain able to parse the feed.
- With valid Collection validators, reconciliation produces the same matched
  classifications without per-Member HTTP `GET`s; network requests scale with
  Collection pages rather than matched Posts. Server-only, local-only, and
  ambiguous inventory rows keep their existing classification semantics.
- Without usable extension metadata, matched Posts are classified through the
  existing Member request path; an individual request failure remains a
  reviewable unclassifiable row, rather than an assumed unchanged Post.
  Malformed and foreign-namespace metadata cannot be used as validators.
- Initial opening and manual refresh show visible in-progress feedback before
  blocking work and finish with a non-misleading status on success or error. A
  refresh failure leaves the old report and selections reviewable.
- Integration and end-to-end coverage for the changed application Collection
  endpoint, plus Emacs pure/live reconciliation tests, demonstrate wire parity,
  pagination, compatibility fallback, classification, and progress. Use
  representative multi-Post evidence to distinguish the reduced request count
  from merely a faster single request.

## Boundaries

- Do not turn reconciliation into an asynchronous operation, change Post
  storage, or invent a separate synchronization API. Do not use preview ETags as
  a substitute for operation-time revalidation.
- Do not change generic Syndication Feeds or AtomPub Member write semantics.
  Compression/conditional ETag behavior is separately tracked by #1641.
- Do not promise an absolute wall-clock target before measurement: the claim is
  elimination of the per-matched-Post request fanout, not a bound on network,
  local file, or server query latency.
