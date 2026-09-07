# Jiff Time Implementation Outline

> Execute with `jaunder-iterate`, delegating isolated slices through
> `jaunder-dispatch`. This outline exists because the issue cuts over a public
> domain/protocol time API, needs a supported-upstream adapter boundary, and
> must retain exact PostgreSQL/SQLite storage and backup compatibility.

## Trigger and scope

In:

- A foundational Jiff domain/codec seam, including the shared dependency and
  bundled-TZDB feature matrix.
- SQLx/dialect compatibility for existing PostgreSQL and SQLite rows.
- Backup wire compatibility and PostgreSQL/SQLite restore interoperability.
- Syndication Feed and AtomPub protocol seams, plus remaining application,
  arithmetic, and named-zone local-time callers.
- Direct-Chrono eradication, the proposed Jiff ADR's accuracy, and gate proof.

Out:

- Replacing Chrono inside third-party dependencies (including Atom/RSS protocol
  crates), full lockfile Chrono eradication, changing schemas, rewriting
  supported-range rows/backups, or changing the established local-time policies.
- Altering the domain meanings recorded by the existing ADRs; the proposed Jiff
  ADR is the only new architecture decision.

## Task outline

- [x] Task 1: Establish the Jiff domain, codec, and feature foundation
  - Contract: one owner updates the root and `common` manifests for the Jiff,
    `jiff-sqlx` 0.2/SQLx 0.9, and target-appropriate bundled IANA-TZDB feature
    matrix, removing SQLx's direct Chrono feature. `common::time` owns
    `UtcInstant` as `jiff::Timestamp`, `PermalinkDate` as civil `Date`, and the
    `UtcInstant` SQLx `Type`/`Encode`/`Decode` bridge; it exposes transparent
    serde, UTC-`Z` display, ordering, `now()`, `value()`, Jiff `From` escape
    hatches, the local conversion helpers, and no general calendar API.
  - Verification: focused `common::time` coverage proves the Jiff-only
    `2024-07-01T16:24Z` fixture, offset canonicalization, `:60` to `:59`,
    serde/display round trips, ordering/now/value/conversions, invalid civil
    dates, civil `Date` endpoints `-009999-01-01` and `9999-12-31`, and
    `Timestamp` endpoints `-009999-01-02T01:59:59Z` and
    `9999-12-30T22:00:00.999999999Z`; supported native, wasm, and browser
    feature-resolution evidence proves every build selects bundled TZDB.

- [x] Task 2: Preserve database codec and legacy-row compatibility
  - Contract: `storage::sql` admits the common-owned `UtcInstant` codec only at
    typed bind sites; the dialect/storage layers consume that codec without a
    schema or data migration. PostgreSQL and SQLite retain their timestamp
    precision and timezone semantics, and SQLite retains its exact established
    timestamp text; no store-wide range scan is introduced.
  - Verification: `#[apply(backends)]` storage coverage reads a supported row
    while a separate out-of-range row remains untouched, then fails only when
    decoding that row. Representative SQLite persisted values match
    pre-migration text byte-for-byte, while PostgreSQL and SQLite each retain
    supported-range instant, civil-date, precision, and timezone meaning.

- [x] Task 3: Preserve backup wire format and four-direction restoration
  - Contract: `storage::backup` and `storage::{sqlite,postgres}::backup` retain
    the existing timestamp wire representation and consume Task 2's established
    codec; they do not add a migration, rewrite, preflight scan, or a
    partial-success guarantee.
  - Verification: pre-migration byte fixtures are readable on both backends; new
    PostgreSQL and SQLite exports match timestamp bytes; PostgreSQL→PostgreSQL,
    PostgreSQL→SQLite, SQLite→SQLite, and SQLite→PostgreSQL restores each assert
    instant, civil date, precision, and timezone meaning. An out-of-range backup
    export or restore fails only when that operation reaches the value.

- [x] Task 4: Cut over Syndication Feed and AtomPub protocol time seams
  - Contract: `host::feed::{FeedMetadata, FeedItem}`, renderers,
    `host::feed::window`, feed ETag time hashing, and direct feed callers use
    domain/Jiff values in one compilable slice. `server::atompub::mapping` owns
    both ingress and egress conversion through supported upstream string/wire
    surfaces; `host::atompub` remains upstream model/XML-only. No Jaunder-facing
    feed or AtomPub signature, model, or conversion imports a Chrono type.
  - Verification: focused feed/AtomPub rendering, hashing/window, and mapping
    coverage compiles first-party callers with domain/Jiff values, preserves
    emitted feed times, and proves both adapter directions isolate any
    Chrono-backed upstream model at the supported wire/string boundary.

- [x] Task 5: Migrate non-feed application callers and retain local-time policy
  - Contract: server, storage, host, client, and Org callers outside Task 4 use
    the Task 1 domain/local-time seam for arithmetic without reintroducing a
    parallel wrapper or parser. HTML `datetime-local` retains its
    minute/optional-second grammar and non-strict browser-normalized gap
    acceptance at `common::time::utc_instant_from_local`; strict conversion and
    Org named-zone parsing retain earlier-fold selection and gap rejection.
  - Verification: native lookup works with system zoneinfo unavailable; wasm and
    browser coverage perform the same lookup and preserve Org zone name/instant
    meaning. Focused local-time/Org tests cover retained outer grammars,
    non-strict spring-forward acceptance, strict/Org gap rejection, and
    earlier-fold selection; non-feed lifecycle, scheduling, expiry, and command
    arithmetic coverage retains its observable behavior.

- [x] Task 6: Complete direct-Chrono eradication and documentation/gate proof
  - Contract: remove direct `chrono`/`chrono-tz` manifest features, imports,
    public types, conversion paths, obsolete tests, and stale implementation
    documentation. Keep `docs/adr/0182-jiff-time-model.md` accurate about its
    limited supersession of ADR-0056, ADR-0072, and ADR-0153; update
    architecture projections only as required by that decision.
  - Verification: manifest and first-party-source scans prove no direct Chrono
    dependency, feature edge, import, type, or conversion remains. Under each
    supported native, wasm, and browser feature configuration, resolved inverse
    dependency checks for `chrono` and `chrono-tz` prove every residual path
    terminates in a third-party crate, not first-party code. Review confirms
    ADR/docs accuracy, then `jaunder-iterate` runs the repository-native cached
    lanes for the completed native, wasm, browser, dual-backend, backup, and
    provenance surfaces.

## Cross-task contracts and ordering

- Task 1 is the sole pre-fanout owner of root/common manifests, the Jiff/
  Jiff-SQLx/TZDB feature matrix, and `common::time`; it publishes the domain,
  SQLx-codec, parser, and local-time contracts consumed by later tasks.
- After Task 1, Task 2 owns only storage SQL/dialect rows, Task 4 owns only feed
  and AtomPub protocol files, and Task 5 owns only non-feed application/Org
  callers; these file sets are non-overlapping and may proceed in parallel. Task
  3 follows Task 2 because it verifies the established codec through backup
  wire/restore paths. Task 6 follows Tasks 3–5 so its provenance proof observes
  the complete cutover.
- Storage owns physical codec admission and dialect representation; backup owns
  archive wire/restore behavior; `server::atompub::mapping` owns the upstream
  wire adapter; callers use Task 1's domain seam and bypass none of them.

## Focused risk checks

- SQLite output is compared as bytes, not parsed-equivalent instants; PostgreSQL
  precision and timezone semantics remain equivalent without schema changes.
- Each backup direction, legacy fixture readability, and delayed out-of-range
  failure is proved independently; no partial-success promise is inferred for an
  export/restore that naturally reaches an invalid value.
- Jiff's broader Timestamp parser and `:60` normalization apply only at the
  `UtcInstant` seam; permalink, HTML-local, and Org outer grammars remain
  deliberately distinct.
- Resolved supported-build features prove bundled TZDB rather than host zoneinfo
  supplies native, wasm, and browser named-zone behavior, including strict gaps
  and ambiguous folds.
- Upstream Atom/RSS Chrono never crosses a first-party protocol seam; final
  manifest/source/inverse-dependency proof distinguishes prohibited direct use
  from residual paths that terminate in third-party crates, including current
  non-protocol roots `axum-embed`, `croner`, and `tokio-cron-scheduler`.
