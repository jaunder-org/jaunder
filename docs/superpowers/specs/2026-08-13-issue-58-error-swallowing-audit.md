# #58 — preserve unexpected failures; observe intentional continuation

Issue: [#58](https://github.com/jaunder-org/jaunder/issues/58). Milestone:
Observability & diagnostics. Absorbs the independently tracked client-side error
concealment in [#898](https://github.com/jaunder-org/jaunder/issues/898) and
[#899](https://github.com/jaunder-org/jaunder/issues/899).

## Summary

Jaunder's typed error pipeline preserves an unexpected source only when the
source reaches it. Production code still contains paths that turn database,
filesystem, browser, subprocess, and configuration failures into a domain miss,
a default value, a successful fallback, or no result at all. The public
operation may then continue or fail under the wrong classification while the
operator loses the cause.

This cycle audits every non-test Rust target, fixes each real instance, and
records one semantic rule:

> An expected validation or domain rejection may be converted into ordinary
> control flow. An unexpected failure must either propagate or, when continuing
> is deliberately correct, emit an operator-visible diagnostic. The diagnostic
> must never expose PII or secrets.

The rule is semantic, not syntactic. `.ok()`, `unwrap_or`, `let _`, `Err(_)`,
and `map_err` each have valid uses and false positives, so no broad source gate
is added. Clippy continues to catch bare unused `must_use` results.

## Evidence and corrected issue premises

The original issue remains directionally correct but its named flows have
drifted:

- `InviteStorage::use_invite` still maps both SQL operations to `NotFound`, but
  has no production caller. Registration uses atomic `create_user_with_invite`,
  which already preserves sqlx failures. The obsolete API is deleted rather than
  repaired.
- `EmailVerificationStorage::use_email_verification` already has
  `Internal(sqlx::Error)`, but preserves only `ColumnDecode` on the claim query;
  every other claim failure and every disambiguation-read failure still becomes
  `NotFound`.
- `AtomicOps::confirm_password_reset` already preserves infrastructure failures
  through `ConfirmPasswordResetError::Internal` on both backends. Its
  closed-pool test does not reach the database because it supplies a malformed
  token, so the behavior needs a valid-token regression test rather than a new
  helper.
- The structured operator carrier now lives in `host::error`, not `web`.

The implementation begins by materializing the audit as
`docs/superpowers/specs/2026-08-13-issue-58-error-swallowing-inventory.md`. That
checked-in manifest is the authoritative population: it records the exact search
recipes and final non-test totals for result-to-option conversions, fallback
combinators, discarded `Result`s/futures, wildcard/catch-all error arms,
source-erasing `map_err`/match conversions, and the issue's named manual flow
audit. Every hit is keyed by file and containing symbol (not a brittle line
number), quotes the relevant expression, and records one disposition:
`expected`, `propagated`, or `continued`. Expected entries state the expected
validation/domain condition; continued entries state the reason, reporting site,
and behavioral proof. The manifest is reconciled to the final tree after all
edits. It is review evidence, not an allowlist or source gate.

## Decisions

### D1 — The audit covers all non-test Rust

The scope is every shipping Rust target in the root workspace, `xtask/`, and
`tools/`. `#[cfg(test)]` modules, test targets, fixtures, benches, and rustdoc
examples are excluded: cleanup and assertion code there has different failure
semantics and does not affect production/operator behavior. D1's checked-in
manifest supplies the complete, reproducible population; aggregate syntax counts
are not completion evidence.

### D2 — “Swallow” means an unexpected failure erased while continuing

Expected invalid input, a domain miss, or a conditional type mismatch may be
converted to `Option`, a domain variant, or a fallback without telemetry. An
unexpected infrastructure, I/O, browser, subprocess, invariant, or decode
failure is not equivalent to absence or validation.

For every unexpected failure:

1. propagate it with its typed source; or
2. if preserving the primary result or an intentional degradation requires
   continuing, report it before continuing.

A comment alone is not observability. It records why continuation is correct;
the report supplies the lost operational signal.

### D3 — Native runtime reporting is one atomic module interface

`host::error::report_swallowed` is the native seam. It accepts bounded
`ErrorKind`, `ErrorClass`, static context, and a PII-reviewed typed source; it
atomically emits:

- a fixed-schema `tracing::warn!` event; and
- `jaunder.errors` with bounded attributes.

Callers cannot emit one side and forget the other. Raw user input is never a
field or source. If a source's display/source chain is not proven PII-safe, the
caller supplies a bounded source kind rather than rendering it.

The event fields are:

- `error.kind`;
- `error.class`;
- `error.disposition = "swallowed"`;
- `telemetry.origin = "server"`;
- static `error.context`;
- PII-reviewed `error.source` or a bounded source kind.

### D4 — `jaunder.errors` counts error events, distinguished by disposition

The existing counter keeps its name and gains:

- `error.disposition = boundary | swallowed`;
- `telemetry.origin = server | client`.

The existing server boundary emits `boundary/server`. Native intentional
continuation emits `swallowed/server`. Authenticated client reports emit
`swallowed/client`.

A root failure may therefore produce both a boundary event and a client
concealment event. They are distinct handling failures, not unique incidents;
dashboards must group/filter by disposition rather than treating an ungrouped
sum as a unique-root-cause count. Metric attributes remain bounded enums/static
mappings; per-site context remains a tracing field, not a metric label.

### D5 — Diagnostic self-failure uses only its fallback channel

The telemetry transport/exporter and panic diagnostics cannot safely report
their own failures through the same path. They use the existing local fallback
(console or stderr/journald) and never recurse through `report_swallowed`, the
authenticated intake, or `jaunder.errors`.

### D6 — The client has one deep reporting interface, not a browser OTel SDK

`client::telemetry::report_swallowed` owns browser-console emission and central
delivery together. It reports unexpected browser-operation failures when the
application deliberately continues: localStorage access/mutation, projector seed
decode, dialog/FormData APIs, and equivalent audited sites. Expected
parsing/downcast mismatch and failures rendered explicitly to the user are not
reported as swallowed.

The client does not install an OTel SDK, expose a collector, add a JavaScript
bundler, or emit OTLP. It sends one Jaunder-owned bounded event to the server;
the server converts it into tracing and the existing OTel metric pipeline.

The interface accepts only closed enums:

- client error kind;
- static client error context;
- bounded source kind.

Arbitrary source text, URLs, routes, usernames, identifiers, form values, and
request bodies cannot enter the wire type. A PII-reviewed full source may be
shown in the local console, but central client telemetry is structurally unable
to carry one.

### D7 — Client delivery is immediate, authenticated, bounded, and best effort

A dedicated raw Axum `POST /api/client-telemetry` route:

- accepts credentials **only** from the `session=` cookie through a dedicated
  browser-session guard; Bearer and Basic/app-password credentials are rejected
  even though the general `AuthUser` extractor accepts them elsewhere;
- accepts exactly one versioned event in an `application/json` body of at most
  1,024 encoded bytes;
- returns `204 No Content` on acceptance;
- returns 401 for a missing, malformed, expired, or unknown session cookie and
  for Bearer/Basic without a valid session cookie;
- returns 400 for malformed JSON, an unsupported version, or an unknown enum
  token; 413 for a body over 1,024 bytes; 415 for a missing/non-JSON content
  type; and 429 when rate-limited;
- returns 500 when session storage itself fails;
- is excluded from client telemetry recursion.

The intake is discoverable and untrusted; cookie-session authentication is the
safety boundary, not endpoint secrecy. A per-user in-memory token bucket admits
a burst of five events and refills one token per minute. The limiter owns a
round-robin identity ring with exactly one entry per bucket. Once a bucket is
full, it is stale after 15 minutes without an intake attempt. Each attempt
advances through at most 64 ring entries, removing stale full buckets and
requeueing the rest, so all retained identities are eventually revisited without
turning one request into an unbounded scan. A newly constructed limiter has no
retained identity or token state; process restart therefore resets this
diagnostic throttle. The dedicated browser-session guard deliberately suppresses
the general `session_validation` application metric for this intake. A 429
therefore produces only the generic HTTP request observability and status: no
tracing warning and no application metric of any kind that an attacker could
amplify.

The client logs locally before delivery. A host-testable scheduler with an
injected transport owns a one-request in-flight cap; a concurrent report is
console-logged and dropped, never queued. The WASM adapter starts a credentialed
`fetch` POST with `keepalive: true`, and the reporting function returns `()`
without awaiting or exposing delivery. There is no persistence, retry, or
batching. Authentication/session-storage outage, response rejection, network
failure, or page termination may lose central delivery but cannot change the
caller's behavior. Delivery failure itself is never reported through the intake.

### D8 — Live incorrect classifications propagate or become explicit failures

The audit applies D2 rather than mechanically adding warnings:

- Delete obsolete `InviteStorage::use_invite`, `UseInviteError`, its generic
  implementation, and dedicated tests.
- In email verification, let every sqlx error become
  `UseEmailVerificationError::Internal`; only `Ok(None)` reaches the row
  classifier. Closed-pool behavior becomes a masked storage failure on both
  backends.
- Password-reset confirmation keeps its current implementation; a valid-token
  closed-pool test proves both backends preserve the infrastructure failure.
- Configured-invalid SMTP or SMTP configuration-read failure aborts startup.
  Only genuinely absent SMTP configuration selects `NoopMailSender`.
- Media file open maps only `io::ErrorKind::NotFound` to HTTP 404; other I/O
  failures preserve the source, emit boundary diagnostics, and return 500.
- Development auto-initialization is SQLite-only and triggers only when
  `open_existing_database` returns SQLite `CANTOPEN` (code 14) **and** metadata
  for the configured database filename returns `NotFound`. An existing but
  unreadable SQLite file, metadata error, malformed URL, pool/connection error,
  or migration failure propagates. PostgreSQL is never auto-initialized by
  `serve`: an existing database is migrated by the normal open, while every open
  failure — including SQLSTATE `3D000` (invalid catalog name) — propagates with
  `create-pg-db` guidance.
- Permalink/timeline projector failures retain their HTTP failure behavior and
  emit operator diagnostics. Tag projection deliberately retains CSR-shell
  recovery, but reports the swallowed projector failure first.
- Scheduled feed status-write failures, cleanup/rollback failures, runtime-file
  removal, backup-size measurement failures, and equivalent intentional
  continuations report once at a useful aggregation level rather than silently
  losing their source or logging one event per recursive entry.

### D9 — Client errors concealed as valid state become visible state

The profile default-format fetch (#898) may not turn a failure into Markdown,
and the post audience fetch (#899) may not render an empty picker
indistinguishably from “no named audiences.” Both render an explicit failure
state and prevent an action based on fabricated data. Because the failures are
no longer swallowed, they do not emit client-swallow telemetry; the server
boundary already records a returned server failure. The issues close with this
cycle.

### D10 — `xtask` and `devtool` fail or write stderr; they do not emit OTel

Short-lived developer/CI tools do not install a meter provider or new tracing
subscriber.

- A correctness-affecting failure propagates and fails the command: unreadable
  policed source, incomplete ADR enumeration, unreadable coverage source or
  markers, and equivalent population loss can never produce a smaller green
  population.
- A legitimate ancillary/cleanup failure preserves the primary command result
  but writes a fixed contextual warning with the typed source to stderr:
  diagnostic-artifact writes/copies, probe-worktree cleanup, ephemeral
  PostgreSQL teardown, timeout cleanup, and equivalent sites.

The warning form identifies the ignored disposition and static context. Tool
stderr is already parked by `devtool run`; no non-exporting counter or tracing
facade is added.

### D11 — No broad syntax gate

A source scanner cannot distinguish `Result` from `Option`, expected validation
from infrastructure failure, fallible from infallible adapters, or adequate
fallbacks from concealment. The measured false-positive population is much
larger than the true defects. The rule lands in ADR-0017, ADR-0011,
`docs/ARCHITECTURE.md`, and `CONTRIBUTING.md`; review and behavior tests enforce
semantics. Existing Clippy `must_use` enforcement remains.

## Acceptance criteria

- **AC1 — Complete classified population.** Before production edits, the D1
  inventory exists at its fixed path with exact search recipes, final non-test
  totals, and one file/symbol/expression row for every hit. After all edits the
  recipes are rerun and reconciled: zero hits are missing, duplicated, or
  unclassified, and no `continued` row lacks its static context, reporting site,
  rationale, and behavioral proof. No unexpected failure in the manifest remains
  silently erased.
- **AC2 — Typed claim failures.** Email verification returns its existing three
  token-domain variants only from successful query results; any sqlx failure is
  `Internal(sqlx::Error)`. A valid-token closed-pool test proves the storage
  classification on SQLite and PostgreSQL.
- **AC3 — Obsolete invite surface removed.** `InviteStorage::use_invite`,
  `UseInviteError`, their implementation, exports, and dedicated tests have no
  declaration or call site. Atomic invite registration remains behaviorally
  unchanged and backend-parametric tests stay green.
- **AC4 — Password reset proof repaired.** Its closed-pool test reaches storage
  with a valid-shaped token and observes `ConfirmPasswordResetError::Internal`
  on both backends; token NotFound/Expired/AlreadyUsed behavior remains intact.
- **AC5 — Native report atomicity.** One host interface emits both the fixed
  WARN event and exactly one `jaunder.errors` measurement with
  `disposition=swallowed, origin=server`; tests assert the event fields and
  metric attributes.
- **AC6 — Boundary metric migration.** Existing boundary failures emit exactly
  one `jaunder.errors` measurement with `disposition=boundary, origin=server`;
  all bounded kind/class mappings remain exhaustive.
- **AC7 — Client wire is bounded.** The client telemetry request type contains
  no free-form string or identifier field. Dual-backend HTTP integration tests
  prove: missing/invalid cookie and Bearer/Basic without a cookie return 401;
  malformed JSON, unsupported version, and unknown enum return 400; an
  authenticated encoded body of 1,024 bytes or less reaches decoding while 1,025
  bytes returns 413; missing/non-JSON content type returns 415; and
  session-storage failure returns 500. No rejected request emits a tracing
  warning or application metric from the intake.
- **AC8 — Intake acceptance.** On SQLite and PostgreSQL, an authenticated valid
  cookie-session event returns 204, emits the fixed WARN fields, and increments
  `jaunder.errors` once with `disposition=swallowed, origin=client`. A valid
  Bearer token and Basic app password are each rejected without the cookie. The
  route uses only its declared browser-session and limiter dependencies, never
  `AppState`.
- **AC9 — Per-user limiting.** With injected time, one user may submit five
  immediate events; the next is 429 until one token refills after one minute.
  Another user has an independent bucket. A full bucket is retained before 15
  idle minutes and removed afterward; the round-robin ring contains exactly one
  entry per bucket, every retained entry is eventually revisited, and one
  cleanup pass inspects no more than 64 entries. A newly constructed limiter has
  no buckets or inherited token state. A 429 emits no tracing warning,
  `session_validation`, `jaunder.errors`, or other application metric; only the
  generic HTTP request observability and status remain. The backend-parametric
  HTTP tests do not sleep.
- **AC10 — Client failure isolation.** Host tests use an injected transport to
  prove the reporting function console-logs first, returns `()` immediately,
  permits one in-flight request, and drops a concurrent report without a queue
  or persistence. Auth rejection, 429, and network failure clear the in-flight
  slot without changing caller state or recursively reporting. A browser test
  observes the console warning and keepalive request start before closing the
  page; delivery after termination is explicitly not guaranteed.
- **AC11 — Real client callers.** Every audited unexpected browser-operation
  failure that deliberately continues calls the single client report interface;
  expected parse/downcast paths do not. A browser flow proves at least one real
  client failure reaches the authenticated intake and appears in captured server
  diagnostics/metrics.
- **AC12 — Concealed UI state removed.** A failed profile default-format fetch
  renders an explicit error and cannot save a fabricated Markdown value. A
  failed named-audience fetch has distinct loading, loaded-empty, and failure
  states; failure cannot present or submit an empty picker as real data. Pure
  host-compiled state/decision tests cover both flows, and Playwright covers
  both user-visible behaviors. This satisfies every acceptance item inherited
  from #898 and #899, including #899's decision that publish remains gated until
  audiences load successfully.
- **AC13 — Mailer startup correctness.** Absent SMTP configuration still starts
  with Noop. Config read failure and configured-but-invalid SMTP each fail
  startup with the typed source preserved; tests distinguish all three.
- **AC14 — Media error classification.** A pure open-error classifier accepts an
  `io::Error`, maps constructed `NotFound` to 404, and preserves every other
  source for a 500 boundary failure. Unit tests use constructed `NotFound` and
  non-`NotFound` errors and assert the source plus one boundary diagnostic; an
  HTTP integration test covers deterministic disappearance, not
  platform-dependent permissions.
- **AC15 — Development initialization correctness.** A missing SQLite filename
  plus SQLite `CANTOPEN` triggers development auto-init. An existing unreadable
  file, metadata failure, non-`CANTOPEN` open error, and migration error
  propagate. PostgreSQL never takes the auto-init branch; SQLSTATE `3D000` and a
  representative connection/migration failure retain their source and
  `create-pg-db` guidance. Classification is unit-tested with constructed
  backend/error inputs; backend integration tests pin the real missing-SQLite
  and missing-PostgreSQL behavior.
- **AC16 — Projector behavior.** Permalink/timeline projection failures retain
  their current failure statuses and emit diagnostics. Tag projection failure
  returns the CSR shell only after one swallowed warning/metric. Tests pin all
  three paths.
- **AC17 — Runtime intentional continuations.** Each audited cleanup,
  rollback/status-write, runtime-file, and measurement continuation either
  propagates or emits one useful aggregated swallowed report. The primary result
  remains unchanged where continuation was selected.
- **AC18 — Tool populations fail closed.** Injected unreadable source/directory
  failures in every affected xtask population scanner fail the step with the
  path/source instead of omitting entries. ADR and coverage enumeration cannot
  turn read failure into an empty population or default marker set.
- **AC19 — Tool ancillary failures are visible.** Injected diagnostic-write,
  cleanup, teardown, and timeout-cleanup failures preserve the primary result
  where specified and write contextual warnings to stderr. Existing JSON/stdout
  contracts remain byte-parseable.
- **AC20 — No telemetry recursion.** Panic-hook, exporter, intake-delivery, and
  rate-limit self-failures use only their designated fallback/request status and
  never call the swallowed reporter or error counter.
- **AC21 — Documentation.** ADR-0017 records the propagate-or-report rule and
  semantic definition; ADR-0011 records metric disposition/origin and the
  authenticated bounded client intake; `docs/ARCHITECTURE.md` projects both;
  `CONTRIBUTING.md` states the actionable rule. No new glossary term is added to
  `CONTEXT.md`.
- **AC22 — Tracker reconciliation.** #898 and #899 are closed by the issue #58
  PR only after every acceptance item from each issue is mapped to delivered
  host and browser evidence; no duplicate implementation remains outstanding.
- **AC23 — Verification.** Targeted dual-backend storage tests;
  backend-parametric client-intake HTTP integration tests using the shared
  `Backend::setup()` fixture; targeted host-compiled client tests and e2e flows;
  and the full `cargo xtask validate` gate are green.

## Out of scope

- A browser OpenTelemetry SDK, document-load/fetch auto-instrumentation, direct
  browser OTLP export, public collector, JavaScript bundler, or dynamic
  traceparent-stamped CSR shell.
- A general client log-ingestion protocol or arbitrary client metric API.
- Durable/offline client telemetry queues or guaranteed delivery.
- A broad syntax/allowlist gate over `.ok()`, `unwrap_or`, `let _`, `Err(_)`, or
  `map_err`.
- Test-only/fixture error-discard cleanup.
- Renaming `jaunder.errors`; its clarified event semantics are recorded instead.
