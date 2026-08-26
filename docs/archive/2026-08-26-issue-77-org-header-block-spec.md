# Issue #77 — Org header metadata block

## Outcome

Jaunder treats the complete leading Org/Jaunder metadata block as structured
input on every Org create and update. Once the whole write is accepted, every
recognized header is removed from the canonical body, including valid mutable
metadata displaced by structured input; unsupported Org directives remain body
content.

## Load-bearing decisions

- The header is the leading sequence of top-level Org keyword elements, ending
  before the first non-keyword top-level element. Org keyword/property names are
  case-insensitive; Org's element parser owns separator and whitespace syntax.
- Structured presence is resolved per field. A supplied valid scalar or
  collection (including an empty collection) wins; otherwise the header may
  supply that field. After both sources are merged, omission retains that
  surface's existing clear, preserve, or default behavior.
- Lifecycle is one merge unit: structured status/publication time or header
  `JAUNDER_STATUS`/`DATE`/`JAUNDER_DATE_TZ` supplies the whole unit, never a
  mixture. Transport defaults do not manufacture structured presence.
- Mutable headers are `#+TITLE`; repeated/comma-separated `#+KEYWORDS`; repeated
  `#+DESCRIPTION`; `#+DATE`; and repeated `#+PROPERTY` values for
  `JAUNDER_AUDIENCE`, plus singleton `JAUNDER_STATUS` and `JAUNDER_DATE_TZ`.
- `TITLE` lines join with newlines then use `PostTitle` validation;
- `DESCRIPTION` lines join with newlines then use `PostSummary` validation.
- Each `KEYWORDS` occurrence flattens its comma-separated terms, drops empty
- comma terms, and must retain at least one valid term; the resulting terms use
- existing `TagLabel` validation, slug deduplication, order, and tag cap. Other
- blank recognized values are invalid.
- Audience values are exactly `public`, `subscribers`, `private`, or
  `named:<numeric-id>`. `private` cannot combine with another target; named IDs
  must exist and belong to the author.
- Duplicate text/list headers compose as above. Duplicate `DATE`, status,
  timezone, or bookkeeping properties reject the request.
- `DATE` is exactly Emacs's inactive `[YYYY-MM-DD Ddd HH:MM]` form with a valid,
  matching weekday and a required IANA `JAUNDER_DATE_TZ`. The earlier instant
  wins at an ambiguous DST fold; a nonexistent DST-gap time rejects. One request
  clock determines future/non-future, with equality non-future.
- A header-sourced lifecycle unit always includes `JAUNDER_STATUS`. `draft`
  permits neither `DATE` nor `JAUNDER_DATE_TZ`; `scheduled` requires both and a
  future instant; `published` either has neither and publishes at the request
  clock, or has both and requires a non-future instant. `DATE` and timezone
  never appear independently. UTC bookkeeping without an effective publication
  instant is invalid.
- `JAUNDER_FORMAT`, `JAUNDER_SLUG`, `JAUNDER_ID`, `JAUNDER_SYNCED`,
  `JAUNDER_SYNCED_AT`, and `JAUNDER_DATE_UTC` are singleton `#+PROPERTY`
  bookkeeping, validated but never authoritative. IDs and slugs use their
  existing typed grammar; ETags compare exactly; sync/UTC times parse as RFC
  3339 instants, with offset spelling irrelevant to instant equality.
- Create rejects `JAUNDER_ID`, `JAUNDER_SYNCED`, and `JAUNDER_SYNCED_AT`.
  `JAUNDER_SLUG` must equal the final stored slug after collision handling,
  `JAUNDER_FORMAT` must be `org`, and `JAUNDER_DATE_UTC` must equal the final
  publication instant.
- Update requires `JAUNDER_ID` to match the target Post,
  `JAUNDER_SLUG`/`FORMAT`/`DATE_UTC` to match the final effective stored values,
  and `JAUNDER_SYNCED` to match the current pre-write content ETag;
  `JAUNDER_SYNCED_AT` is syntax-only. AtomPub `If-Match` remains independently
  required when supplied.
- Malformed/conflicting metadata, a foreign audience, and metadata-only input
  reject atomically. Web reports Validation, except stale sync is Conflict;
  AtomPub reports 400, except stale sync is 412. Errors do not reveal whether a
  foreign named audience exists.
- This evolves ADR-0024's deliberate no-full-header-parsing decision.

## Acceptance

- Creating or updating an Org Post recognizes every supported keyword or
  property in its leading metadata block regardless of case, and does not treat
  a keyword after the first non-keyword top-level element as header metadata.
- For each mutable field, an explicitly structured value wins over header input;
  header input fills only its absence; and omission from both retains the
  existing surface-specific update/default result.
- Valid title, keywords, description, date/time zone, status, and audience
  headers produce the corresponding Post state, including the duplicate and
  multiline composition rules above.
- After the whole request succeeds, every recognized header is absent from the
  saved canonical body, including valid mutable metadata that lost precedence;
  unknown Org directives remain unchanged. A rejected request leaves the prior
  Post unchanged.
- Invalid date/time-zone or status/date combinations, prohibited duplicate
  bookkeeping/singleton fields, invalid audience syntax, private-plus-other
  audiences, missing/foreign named audiences, and invalid bookkeeping fail the
  request atomically.
- Create rejects `JAUNDER_ID`, `JAUNDER_SYNCED`, or `JAUNDER_SYNCED_AT`; rejects
  `JAUNDER_SLUG`, `JAUNDER_FORMAT`, or `JAUNDER_DATE_UTC` when it disagrees with
  the effective derived value; and update rejects any mismatched derivable
  current/target metadata, stale ETag, or invalid sync time.
- Metadata-only Org input is rejected without changing stored content.
- Existing title, summary, tag, and audience omission semantics remain
  unchanged.

## Boundaries

- No new domain vocabulary, metadata aliases, compatibility paths, or changes to
  non-Org create/update formats are introduced.
- This spec records behavior only; it does not prescribe private module seams or
  implementation order.
