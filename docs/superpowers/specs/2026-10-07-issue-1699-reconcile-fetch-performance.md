# Issue #1699: fast Post-link localization and opt-in client diagnostics

Issue: <https://github.com/jaunder-org/jaunder/issues/1699>

## Outcome

Fetching selected Posts through the Emacs Protocol Client avoids all-to-all
filesystem validation when restoring Local Post Links. Off-by-default,
timestamped diagnostics identify slow work throughout the client without
exposing credentials or authored content.

## Load-bearing decisions

- Fix the confirmed replacement-construction bottleneck: 450.616s inside
  451.434s staging, versus 0.397s Media materialization; see
  [the diagnosis](../research/2026-10-07-issue-1699-reconcile-fetch-performance.md).
  Match identity before filesystem validation; unique inventories must not
  require their Cartesian product. Ambiguity never selects an arbitrary winner.
- Preserve [ADR-0201](../../adr/0201-emacs-local-post-link-round-trip.md): exact
  canonical href and local ID/slug/filename agreement, current same-root file
  checks, unchanged unproven links, no guessed destinations or extra downloads.
- Preserve [ADR-0200](../../adr/0200-revalidated-matched-post-pull.md) and
  [ADR-0211](../../adr/0211-emacs-reconciliation-conflict-resolution.md): staged
  replacement, per-Post fresh Collection uniqueness and Member ETag checks,
  unchanged local bytes/identity, clean visiting buffers, explicit conflicts,
  recoverable installation, and honest partial/unknown outcomes. Do not share
  freshness across selected Posts; the roughly nine-second scan remains
  separate.
- One client-wide diagnostic option defaults off. Disabled work creates no
  buffer/events, samples no diagnostic clock, computes no diagnostic fields,
  invokes no diagnostic formatter, and changes no correlation/span state.
- Enabled events go only to `*Jaunder Debug*`: no automatic display, saved
  files, or replacement of ordinary progress/warnings. Every inventory label
  below is mandatory. Events never contain arguments, raw URLs/paths, usernames,
  headers, credentials/tokens, authored titles/bodies, response bytes, or
  free-form errors.
- Each enabled outermost operation creates a session-unique correlation ID.
  Children inherit it, get distinct span IDs, and identify their parent;
  standalone lower-level calls create roots. Start/terminal events share IDs and
  label, with exactly one terminal per uninterrupted logged start, including
  errors and `quit`. Pairing excludes disabled/output-failed periods and records
  discarded by clear/eviction. Preserve return values, signals and cancellation.
- Retain evidence across quitting/burying/killing reconciliation, authoring and
  merge scratch/Ediff views, and across disabling logging. Provide show, clear
  and disable commands. Debug-buffer killing while enabled permits recreation at
  the next event; its read-only view buries on quit. Clear empties an existing
  buffer (no header/notice), resets eviction accounting, preserves the option,
  and creates no buffer. Correlation/span IDs must not be reused after clear.
- Retain at most 10,000 newest complete event lines plus one first-line eviction
  marker giving the cumulative discarded count since clear/buffer creation. The
  marker is not an event and is at most 128 ASCII bytes excluding newline.
  Output failure preserves operation outcomes and emits a fixed non-sensitive
  warning through the existing warning channel, without recursive diagnostics.

## Diagnostic fields and bounds

Every event permits only `at`, `correlation`, `span`, `parent`, `label`,
`phase`, `elapsed-ms`, `outcome`, and the optional keys listed below. `at` is a
UTC ISO 8601 millisecond timestamp; `phase` is `start` or `end`. IDs are
internally assigned 1–32 lowercase ASCII alphanumeric/hyphen characters; roots
omit parent. Labels are exactly the expanded inventory set. End events include
outcome (`success`, `error`, `cancelled`) and nonnegative elapsed integer
milliseconds. Successful calls may still report a blocked/no-op business
decision.

Counts (`count`, `bytes`, `page`, `members`), elapsed milliseconds, and eviction
counts range from 0 to 9,007,199,254,740,991; larger values saturate at that
bound. `http-status` is an integer 100–599; `reused` is boolean. Optional enum
values:

| Key           | Allowed values (each also permits `unknown`)                                                                                                                            |
| ------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `method`      | `GET`, `HEAD`, `POST`, `PUT`, `DELETE`                                                                                                                                  |
| `format`      | `org`, `markdown`, `html`, `atom`                                                                                                                                       |
| `action`      | `new`, `complete`, `cancel`, `discard`, `publish`, `draft`, `create`, `recover`, `update`, `push`, `pull`, `keep-local`, `keep-remote`, `merge`, `delete`, `refresh`    |
| `eligibility` | `eligible`, `ineligible`                                                                                                                                                |
| `decision`    | `proceed`, `blocked`, `no-op`, `retry`, `recovered`, `partial`, `remote-unknown`                                                                                        |
| `reason`      | `invalid`, `stale`, `ambiguous`, `ineligible`, `missing`, `unsafe-path`, `modified`, `conflict`, `transport`, `decode`, `io`, `partial`, `remote-unknown`, `unexpected` |

Unknown enum inputs map to literal `unknown`, never their source spelling.
Absent optional fields stay absent. Unknown keys/labels, wrong types, invalid
status values or oversized IDs/events are rejected without serializing their
values, using the diagnostic-failure policy above. Each event is one ASCII line
of at most 1,024 bytes excluding newline; no payload-derived text is admissible.

## Diagnostic operation inventory

Braces expand into distinct mandatory labels. Time each listed boundary,
including standalone calls. Mark/region selection, presentation-only accessors,
and individual property reads inside an aggregate scan need no separate timer.

| Label(s)                                                                     | Required boundary                                                                |
| ---------------------------------------------------------------------------- | -------------------------------------------------------------------------------- |
| `config.resolve`, `auth.lookup`                                              | Root/account resolution; App Password lookup, never its value                    |
| `author.new`, `author.{complete,cancel}`                                     | New-Post preparation and completion/cancellation                                 |
| `publish.{post,draft}`, `publish.validate`                                   | Publish/save-draft commands and source validation                                |
| `publish.{create,recover,update}`, `publish.checkpoint`                      | Create/recovery/conditional update; confirmed metadata write-back and rename     |
| `delete.post`                                                                | Single-Post deletion                                                             |
| `report.{open,refresh}`, `inventory.build`                                   | Reconciliation entry/refresh and complete identity join                          |
| `inventory.{local,collection}`, `inventory.page`                             | Local scan, complete pagination, each page request/parsing/validation            |
| `reconcile.batch`, `reconcile.row`                                           | Confirmed batch and each selected row, with action/decision                      |
| `conflict.{local,remote,merge}`, `merge.{stage,finish,cancel,discard}`       | Explicit resolution commands, staging/Ediff setup, scratch lifecycle             |
| `pull.stage`, `pull.{revalidate,preflight,install}`                          | Member staging, final remote/local checks, atomic install/rename                 |
| `transport.request`, `service.{read,parse}`                                  | Authenticated requests; Service Document retrieval/validation                    |
| `atom.{parse,serialize}`, `org.{parse,serialize}`, `member.{identity,parse}` | Entry/authored-source transformations; pulled identity/representation validation |
| `post-link.{publish,pull,evidence}`                                          | Forward/reverse mapping and replacement-proof construction                       |
| `media.{plan,materialize,apply}`, `media.{upload,download}`                  | Localization, verified acquisition/reuse, source replacement, network transfer   |
| `media.{path,hash,verify}`                                                   | Target/directory checks, literal-file hashing, copy/response validation          |

## Acceptance

- Real reverse-localization regressions at 100 and 1,000 remote/local Posts
  include public canonical alternate hrefs. An unrelated image URL requires no
  more than one filesystem target validation per Post; valid unique Post links
  still localize. Work counts, not host-dependent timing thresholds, gate this.
- The selected-pull fixture gains public permalink metadata while retaining real
  staging/install/refresh and existing request-count/freshness guarantees.
- Cover duplicate IDs/hrefs, stale/mismatched ID/slug/filename,
  missing/out-of-root files, partial inventories, unproven URLs, and unchanged
  non-link bytes. Existing pull, conflict, Media and safety tests stay green.
- Disabled tests use fail-on-call clock/formatter/field/buffer/event sentinels
  and unchanged diagnostic-state assertions, not general allocation claims.
  Enabled tests prove timestamps, elapsed bounds, root/child/standalone pairing,
  error/quit behavior, and preservation of results and signaled conditions.
- A delivered table maps every expanded label to a pure/live test exercising its
  actual boundary. Every field producer has secret/content sentinel proof or
  only literal enum/size fields. Test every allowlist/bound, including unknown
  enum mapping and rejection without leakage; representative-only proof fails.
- Buffer tests prove no automatic display, show/clear/disable semantics,
  frontend/disable retention, recreation, exact eviction accounting/line bounds,
  and non-interference even when diagnostic output or its warning fails.
- README covers option, capture/sharing, lifecycle, retention and Collection
  cost. Pure/live tests, gates and authoritative Emacs coverage verify the tree;
  production replay confirms timing. After approval, an outline addresses
  privacy, cancellation, instrumentation and diagnostic-failure risks.

## Boundaries

No server/schema/protocol changes, async redesign, shared freshness, new Media
behavior or saved/private logs. Temporary `.xtask` probes do not ship.
