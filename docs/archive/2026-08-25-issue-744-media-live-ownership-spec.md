# Issue #744 — verify media URL ownership live

## Outcome

An unforced deletion is refused only when a Post currently points readers at
media served by this Jaunder instance, or when ownership cannot be determined.
A foreign URL whose path happens to resemble a local media path no longer
protects the local file.

Every Jaunder response identifies its persistent instance with
`X-Jaunder-Instance`. Absolute references are checked with bounded live HTTP
HEAD probes immediately before deletion; the storage decision remains atomic
against concurrent Post edits.

## Load-bearing decisions

- Sanitized rendered HTML remains authoritative. Rendering and extraction stay
  pure and receive no configuration or network dependency.
- A persisted reference row key is exactly
  `(post_id, source, sha256, filename, reference_kind, reference_form)`. Parser
  kinds are `local`, `absolute`, or `scheme_relative`; `legacy` exists only
  during migration/backfill.
- Relative and root-relative references are intrinsically local and never
  probed.
- Absolute forms retain the complete query-bearing probe target.
  Scheme-relative forms retain complete authored authority, explicit port, path,
  and query. Probe resolution inherits only the current canonical
  `site.base_url` scheme—never its authority or path. No configured base URL is
  unknown and fails closed.
- Each database owns one persistent canonical random UUID, created by atomic
  insert-if-absent/read-back on first open. Backup and restore preserve it; two
  simultaneous clones conservatively recognize each other as the same logical
  instance.
- Outer middleware replaces any existing identity header and emits exactly one
  canonical `X-Jaunder-Instance` value on every response, including errors,
  fallbacks, and method-not-allowed responses.
- Probing uses one shared ordinary reqwest client and one HEAD operation per
  target, with a request timeout, at most eight concurrent probes, and a 10 s
  whole-operation deadline. Identical resolved targets are network-deduplicated;
  unstarted or unfinished targets are unknown.
- Reqwest owns DNS, configured proxies, redirects, TLS, and connection pooling.
  Jaunder performs no IP classification, DNS resolution, socket pinning, or
  manual redirect handling. Post authors are trusted rather than treated as
  hostile network-input authors. If a product later disallows an address class,
  it must reject that URL at the authoring validation boundary, not while
  deleting media.
- A final response with exactly one canonical matching instance UUID is owned.
  True absence or exactly one canonical different UUID is foreign. Duplicate,
  list-valued, malformed, or noncanonical identity headers and request failures
  are unknown and fail closed.
- Public UUID is identification, not authentication. Copying it can cause only
  conservative refusal; `force` remains the owner escape hatch.
- `ProvenForeignReference` has a resolver-only constructor and contains the
  complete persisted row key plus the expected `InstanceId`. Evidence is exact:
  it cannot cross Post, media identity, kind, query-bearing form, or instance
  identity, and is never cached across deletion requests.
- Resolution loads a bounded global exact-reference snapshot for the media
  across every User/Post once: at most 128 rows plus a sentinel. The resolver
  counts every examined row toward the cap; unexamined rows have no evidence and
  remain live. Owner reporting is a subset of that global snapshot. Force
  bypasses only owner refusal; global last-row/reclamation uses every row.
- Storage exempts only current rows exactly present in the proven-foreign set.
  Local, legacy, unknown, owned, near-match, and concurrently inserted/unprobed
  rows refuse. Reporting, owner refusal, global protection, and reclamation use
  the same evidence set.
- PostgreSQL uses transaction-scoped `pg_advisory_xact_lock` in one stable
  media-key namespace and global sort order. Create locks proposed identities;
  update locks the sorted union of persisted old and proposed new identities;
  delete/reclaim locks the target before conditional SQL. SQLite uses its
  immediate/single-writer discipline. Network probes happen before locks.
- The deletion decision stays one `DELETE … WHERE … NOT EXISTS … RETURNING`
  statement; no network work occurs inside a database transaction.

## Acceptance

- Resolver tests cover matching, different, absent, duplicate, list-valued,
  malformed, and noncanonical identity headers, plus request-failure unknown.
- Probe-budget tests prove exact target deduplication, eight-request concurrency,
  and the 10 s operation bound with unfinished rows unknown.
- Scheme-relative tests prove only scheme inheritance; base host/path changes do
  not affect authored authority/port/path/query.
- A local HTTP integration test proves the shared reqwest client sends HEAD and
  classifies an absent identity as foreign.
- Both backend migrations preserve every identity. Backfill leaves no legacy on
  success, rolls back wholly on failure, and retries on a later open.
- Exact evidence cannot cross Post/media/kind/query/InstanceId; every near-match
  and concurrent new row refuses.
- PostgreSQL tests cover insert-vs-delete, update old/new union, opposite
  multi-key update order without deadlock, rollback release, and reclaim parity;
  SQLite covers the same observable races under its writer discipline.
- Web and AtomPub use one global reference/probe/evidence snapshot through
  delete, reclamation, and web reporting.

## Boundaries

- No origin-candidate collection or configured-origin SQL predicate.
- No request-derived proxy scheme; scheme-relative URLs use the configured
  canonical site scheme selected by the user.
- No ETag comparison, nonce/HMAC identity, cached probe results, browser UI, or
  deletion-time address policy.
