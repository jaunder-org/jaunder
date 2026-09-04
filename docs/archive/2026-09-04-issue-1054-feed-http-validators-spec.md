# Complete Syndication Feed HTTP Validators

Issue: #1054

## Outcome

Every cached RSS, Atom, and JSON Syndication Feed exposes validators for the
exact stored representation. Conditional GET and HEAD requests follow RFC 9110,
including weak `If-None-Match` comparison, list and wildcard handling,
`If-Modified-Since` precedence, and body-free `304 Not Modified` responses with
current validator and cache metadata.

Regenerating semantically identical input preserves the representation body,
strong ETag, and whole-second modification time, including when identical inline
and background regenerations race. A changed input that wins the existing
generation-fenced commit installs a new identity and modification time.

## Load-bearing decisions

### Representation identity

- A private semantic-input fingerprint covers this ordered tuple:
  - Syndication Feed format and that format's serializer revision;
  - feed title, optional description, canonical URL, self URL, and optional
    WebSub hub URL;
  - each item in serialization order, including ID, title, permalink, optional
    summary, rendered HTML, publication time, update time, and tags in their
    serialization order.
- The derived representation-modification time is excluded from the semantic
  fingerprint. It is selected only after storage determines whether the semantic
  tuple changed.
- RSS, Atom, and JSON own independent explicit serializer revisions. A change
  that can alter one format's bytes increments that format's revision even when
  its semantic fields do not change.
- The public strong ETag covers the complete final serializer input tuple:
  semantic inputs, format-specific serializer revision, and the selected
  feed-level modification time. Hashing the rendered body is not the identity
  contract.
- Atom `feed.updated` and RSS `channel.lastBuildDate` use the persisted
  representation-modification time also sent as `Last-Modified`, not the newest
  item timestamp or regeneration wall time. JSON Feed gains no feed-level
  timestamp; each JSON item's `date_modified` remains that item's update time.

### Atomic cache ownership

- Feed-cache storage persists the semantic-input fingerprint alongside the
  representation, ETag, modification time, and generation time.
- Cache replacement compares the fingerprint atomically within the
  generation-fenced write:
  - a matching fingerprint preserves the stored body, ETag, and modification
    time while advancing `generated_at`;
  - a different fingerprint installs the newly rendered body, ETag, and current
    whole-second modification time.
- The commit returns the effective stored row. An inline cache-miss response
  therefore serves the winner of a concurrent same-identity write rather than an
  uncommitted candidate with a different timestamp.
- Modification time is truncated to whole seconds before rendering, hashing,
  persistence, or comparison. Two genuine changes in one second may share the
  weak `Last-Modified` value; their strong ETags remain distinct.
- The dual-backend migration requires a semantic fingerprint for every new cache
  row and invalidates all pre-policy cached rows. It does not attempt to infer
  complete identity from legacy ETags.

### Conditional request evaluation

- Preconditions apply to GET and HEAD after selecting or regenerating the
  current representation.
- Presence of any `If-None-Match` field suppresses `If-Modified-Since`
  evaluation, whether the ETag condition matches, does not match, or is
  malformed.
- Repeated `If-None-Match` field lines are combined in arrival order. The parser
  accepts a comma-separated entity-tag list, optional whitespace, a reasonable
  bounded number of empty list elements, or `*` by itself.
- Entity-tags use RFC 9110 weak comparison: matching opaque tags compare equal
  regardless of a request tag's `W/` prefix. Any matching list member selects
  `304 Not Modified`; `*` matches the selected cached representation.
- Entity-tag opaque values are parsed as RFC 9110 bytes, including permitted
  `obs-text`; they are not required to be UTF-8. Malformed non-empty members,
  invalid quoting, bytes outside the entity-tag grammar, and `*` mixed with list
  members cannot produce 304. They remain present conditions, so they also
  cannot fall through to `If-Modified-Since`.
- With no `If-None-Match`, a valid single `If-Modified-Since` date selects 304
  when the representation modification time is not later than the request date.
  Invalid or multi-member values are ignored.
- HTTP dates are parsed in all three RFC 9110 forms and emitted as IMF-fixdate
  with `GMT` and one-second precision.

### Response contract

- A nonmatching GET returns 200 with the complete stored body, Content-Type,
  ETag, Last-Modified, and `Cache-Control: public, max-age=300`.
- A nonmatching HEAD returns 200 with GET-equivalent headers and no body.
- A matching GET or HEAD returns 304 with no body or trailers and the current
  ETag, Last-Modified, and Cache-Control. Content-Type is omitted from 304.
- Existing public Syndication Feed routes and cache-miss regeneration behavior
  remain unchanged.

## Acceptance

- Determinism tests cover all three formats: identical complete inputs preserve
  the ETag; changing each metadata field, each item field, item order, tag
  order, format, serializer revision, or derived modification time changes it.
- Regeneration tests cover empty feeds, metadata-only changes, item removal,
  transition to and from empty, and byte-identical no-op regeneration.
- Dual-backend storage tests prove matching-fingerprint preservation,
  changed-fingerprint replacement, whole-second persistence, advancing
  `generated_at`, returned effective rows, and legacy-cache invalidation.
- A concurrent same-fingerprint regeneration test proves one stable body, ETag,
  and modification time is observed and persisted.
- Conditional-request tests cover exact and weak ETag matches, nonmatches,
  multiple tags, repeated fields, whitespace, bounded empty members, wildcard,
  wildcard mixed with members, malformed tags, valid `obs-text` alongside a
  matching tag, invalid bytes, and `If-None-Match` precedence over valid and
  invalid `If-Modified-Since`.
- Date tests cover IMF-fixdate and both obsolete accepted input forms, invalid
  and multi-member dates, second-boundary comparisons, and IMF-fixdate output.
- Route-level tests cover the full GET/HEAD status, header, and body matrix for
  200 and 304 responses on both SQLite and PostgreSQL.

## Boundaries

- HybridWindow membership and age-expiry policy remain governed by ADR-0139.
- `Cache-Control: public, max-age=300` is unchanged.
- This work does not add range, mutation preconditions, or validators to other
  HTTP resources.
- This work does not hash rendered bodies or make item timestamps stand in for
  representation modification time.
- Ordering different-fingerprint regenerations derived from different Post
  snapshots is existing publication-consistency policy, not part of HTTP
  validator identity. This work preserves the existing generation fence and does
  not introduce a new Post snapshot version.
