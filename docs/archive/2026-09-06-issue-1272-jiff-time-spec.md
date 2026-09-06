## Outcome

Jaunder represents first-party instants, civil dates, and named-zone operations
with Jiff rather than Chrono while preserving the observable storage, backup,
and protocol behavior that existing deployments rely on.

The migration deliberately adopts Jiff's temporal parsing and supported range,
while retaining the domain concepts and backend compatibility guarantees at
Jaunder's public and persistence boundaries.

## Load-bearing decisions

- `UtcInstant` remains Jaunder's domain instant type and wraps
  `jiff::Timestamp`. It retains transparent RFC 3339 serde, `Display` and
  `FromStr`, chronological ordering, `now()`, `value()`, and the established
  `From`-style Jiff conversion escape hatches; it does not expose a new
  application-specific timestamp representation.
- `PermalinkDate` is a Jiff civil `Date`; its fixed `YYYY-MM-DD` outer grammar
  and civil-date permalink semantics remain unchanged and must not acquire an
  instant or time-zone component.
- Public Syndication Feed time seams accept and return domain/Jiff time types,
  not Chrono types, so first-party callers do not need Chrono to use those
  seams.
- All direct, first-party dependencies on `chrono` and `chrono-tz` are removed
  from manifests, feature sets, source imports, type signatures, and first-party
  conversion code.
- Chrono may remain only as an unavoidable transitive implementation dependency
  of third-party crates; that allowance does not permit a direct dependency or a
  first-party Chrono type to cross a Jaunder seam.
- Upstream protocol models that remain Chrono-backed are adapted at their
  boundary through their supported string surfaces or equivalent upstream
  wire-format conversion; Chrono must not be reintroduced into Jaunder domain or
  public protocol APIs.
- `UtcInstant` text and serde parse directly as Jiff `Timestamp` input:
  Jiff-supported Temporal timestamp spellings are accepted there even when the
  former Chrono parser rejected them. `PermalinkDate` retains its fixed
  `YYYY-MM-DD` outer grammar; HTML `datetime-local` retains its existing
  minute/optional-second outer grammar; and Org retains its structured
  `DATE`/weekday/time/time-zone grammar before its civil `DateTime` and
  named-zone resolution.
- Canonical serialized `UtcInstant` output is UTC with the `Z` designator.
- A parsed `UtcInstant` leap-second `:60` is normalized to `:59` in the
  resulting Jiff value and canonical output, matching Jiff's behavior rather
  than preserving an unrepresentable leap-second spelling.
- Each time-domain type uses its native Jiff range: civil `Date` spans
  `-009999-01-01` through `9999-12-31`, while `Timestamp` spans
  `-009999-01-02T01:59:59Z` through `9999-12-30T22:00:00.999999999Z`. Legacy
  values outside their type's range fail only when an affected operation
  decodes, exports, or restores them.
- The migration performs no global preflight scan for out-of-range persisted
  values and does not reject otherwise unaffected stores or backups merely
  because such a value may exist elsewhere.
- Bundled IANA TZDB in every supported build, including wasm and browser, makes
  named-zone conversion independent of host system zone files.
- Org named-zone timestamps resolve with that data on native, wasm, and browser
  targets; their observable zone-name and instant semantics remain intact.
- The HTML `datetime-local` create-form's non-strict
  `common::time::utc_instant_from_local` accepts browser-normalized
  spring-forward gaps. `strict_utc_instant_from_local` and Org named-zone
  parsing reject gaps and retain earlier-fold selection.
- PostgreSQL and SQLite schemas, stored precision, and timezone semantics are
  preserved. Existing database contents remain readable without a schema or data
  migration solely because Jiff replaces Chrono.
- SQLite timestamp text remains byte-for-byte identical to the established
  representation for values within the supported range, including its UTC
  spelling and fractional-second precision.
- SQLx storage integration uses `jiff-sqlx` 0.2 with SQLx 0.9 and a deliberate
  bridge where needed to preserve both PostgreSQL and SQLite contracts; a
  first-party Chrono dependency is not retained as that bridge.
- Backup export preserves the established timestamp wire representation for
  supported-range values; every PostgreSQL/SQLite source-to-target pair
  preserves instant, civil date, precision, and timezone meaning. Existing
  supported-range backups retain their representation and restore behavior;
  out-of-range values fail only at the decode, export, or restore operation that
  reaches them.
- The new proposed, numberless ADR draft at `docs/adr/drafts/jiff-time-model.md`
  owns this architecture decision. It supersedes only the Chrono-specific
  implementation portions of ADR-0056, ADR-0072, and ADR-0153 (including
  ADR-0153's obsolete SQLx conclusion); their retained domain, module, and
  boundary decisions remain in force.

## Acceptance

- The delivered dependency graph and first-party source contain no direct
  `chrono` or `chrono-tz` dependency, import, public type, or conversion path.
  Every residual Chrono reverse path terminates in a third-party crate; current
  native roots include `atom_syndication`/`rss`, `axum-embed`, `croner`, and
  `tokio-cron-scheduler`.
- `UtcInstant` is backed by `jiff::Timestamp` and exercises transparent RFC 3339
  serde, `Display`/`FromStr`, chronological ordering, `now()`, `value()`, and
  its `From`-style Jiff escape hatches. `PermalinkDate` is backed by Jiff civil
  `Date` while retaining `YYYY-MM-DD`.
- The public Syndication Feed time interfaces compile and are exercised with
  domain/Jiff types without exposing Chrono types to first-party callers.
- Parser coverage identifies each input seam: direct Jiff `Timestamp` parsing
  for `UtcInstant` text/serde; retained `YYYY-MM-DD` for `PermalinkDate`;
  retained minute/optional-second HTML-local grammar; and retained structured
  Org `DATE`/weekday/time/time-zone grammar. It includes the exact Jiff-only
  Temporal fixture `2024-07-01T16:24Z`, UTC-`Z` canonical instant output, and
  `:60` normalization to `:59`.
- Boundary coverage demonstrates each time-domain type's native Jiff range:
  `PermalinkDate` accepts `-009999-01-01` and `9999-12-31`, while `UtcInstant`
  parses and displays `-009999-01-02T01:59:59Z` and
  `9999-12-30T22:00:00.999999999Z`. On PostgreSQL and SQLite, a supported row
  remains readable while a separate out-of-range row exists until that row is
  decoded. Backup fixtures likewise show unaffected entries/operations remain
  usable until the operation reaches the out-of-range value; a whole export or
  restore MAY fail when it naturally reaches that value, without a promised
  partial-success result.
- Dependency and feature evidence proves an always-bundled IANA TZDB for every
  supported build. Native coverage performs a named-zone lookup with system
  zoneinfo unavailable; wasm and browser coverage perform the same lookup and
  demonstrate the same Org named-zone instant and zone-name meaning.
- Local-time coverage demonstrates acceptance of the browser-normalized
  spring-forward result at non-strict `common::time::utc_instant_from_local`,
  plus earlier-fold selection and gap rejection at
  `strict_utc_instant_from_local` and Org named-zone parsing.
- Backup coverage uses pre-migration byte fixtures readable on both backends,
  byte-for-byte timestamp fixtures for newly exported PostgreSQL and SQLite
  backups, and every restore pair: PostgreSQL→PostgreSQL, PostgreSQL→SQLite,
  SQLite→SQLite, and SQLite→PostgreSQL. Each pair asserts instant, civil-date,
  precision, and timezone meaning.
- SQLite compatibility coverage demonstrates exact established timestamp text
  for representative persisted values, not merely equivalent parsed instants.
- Protocol adapter coverage demonstrates conversion through upstream-supported
  wire/string surfaces when Atom/RSS models are Chrono-backed and demonstrates
  that no Jaunder-facing protocol seam exposes Chrono.
- The proposed ADR draft exists at the stated path, records these architecture
  commitments and its relationship to ADR-0056, ADR-0072, and ADR-0153, while
  leaving those historical ADR records intact.

## Boundaries

- This issue does not change Jaunder's ubiquitous language: `UtcInstant`,
  `PermalinkDate`, feeds, backups, and named-zone timestamps keep their
  established domain meanings.
- This issue does not remove or fork Chrono from third-party dependencies or
  require full lockfile Chrono eradication; it removes only Jaunder's direct,
  first-party Chrono use.
- This issue does not introduce a persistence schema migration, rewrite
  supported-range database rows, rewrite existing supported-range backups, or
  add a store-wide out-of-range preflight scan.
- This issue does not alter the named local-time seams: non-strict
  `utc_instant_from_local` acceptance, or strict/Org earlier-fold and gap
  behavior.
- This spec records externally reviewable behavior and invariants only; it does
  not prescribe implementation sequencing, internal task breakdown, or an
  alternative architecture record.
