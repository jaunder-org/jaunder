# Publication-Time Projection Case Table

Issue: #1276 Status: Approved

## Outcome

The pure Protocol Client publication-time projection contract is expressed by
one ERT test over a complete local case table. Every lifecycle, timezone, and
missing-value case reports its stable representation-case identity when it
fails.

## Load-bearing decisions

- One ERT test loops the eight cases; it does not generate eight separate
  `ert-deftest` registrations.
- Each row carries the exact former test-name suffix as a symbol, complete Org
  source, and the expected `jaunder-entry-published` value.
- Expected values are either an exact RFC-3339 UTC string or nil, and one
  `equal` assertion handles both forms without branching.
- The shared assertion is wrapped in `ert-info` containing the row label so a
  failure identifies the representation case.
- The numeric-offset regression and the publish-now, draft, and missing-date nil
  semantics retain explanatory comments beside their rows.
- The seam remains `jaunder--org->atom` through `jaunder-test--entry`, observed
  as the pure `jaunder-entry` struct before XML serialization, per ADR-0042.

## Normative cases

In the source column, `\n` denotes one newline byte.

| Label                             | Complete Org source                                                                                                         | Expected published value |
| --------------------------------- | --------------------------------------------------------------------------------------------------------------------------- | ------------------------ |
| `published-iana-dst-summer`       | `#+DATE: [2026-07-01 Wed 09:00]\n#+PROPERTY: JAUNDER_STATUS published\n#+PROPERTY: JAUNDER_DATE_TZ America/New_York\n\nB\n` | `2026-07-01T13:00:00Z`   |
| `published-iana-dst-winter`       | `#+DATE: [2026-01-01 Thu 09:00]\n#+PROPERTY: JAUNDER_STATUS published\n#+PROPERTY: JAUNDER_DATE_TZ America/New_York\n\nB\n` | `2026-01-01T14:00:00Z`   |
| `published-numeric-offset-string` | `#+DATE: [2026-07-01 Wed 09:00]\n#+PROPERTY: JAUNDER_STATUS published\n#+PROPERTY: JAUNDER_DATE_TZ -0500\n\nB\n`            | `2026-07-01T14:00:00Z`   |
| `published-numeric-offset-colon`  | `#+DATE: [2026-07-01 Wed 09:00]\n#+PROPERTY: JAUNDER_STATUS published\n#+PROPERTY: JAUNDER_DATE_TZ -05:00\n\nB\n`           | `2026-07-01T14:00:00Z`   |
| `published-scheduled`             | `#+DATE: [2026-07-01 Wed 09:00]\n#+PROPERTY: JAUNDER_STATUS scheduled\n#+PROPERTY: JAUNDER_DATE_TZ America/New_York\n\nB\n` | `2026-07-01T13:00:00Z`   |
| `published-publish-now-is-nil`    | `#+PROPERTY: JAUNDER_STATUS published\n\nB\n`                                                                               | nil                      |
| `published-draft-is-nil`          | `#+DATE: [2026-07-01 Wed 09:00]\n#+PROPERTY: JAUNDER_STATUS draft\n#+PROPERTY: JAUNDER_DATE_TZ America/New_York\n\nB\n`     | nil                      |
| `published-missing-date-is-nil`   | `#+PROPERTY: JAUNDER_STATUS scheduled\n#+PROPERTY: JAUNDER_DATE_TZ America/New_York\n\nB\n`                                 | nil                      |

The raw `-0500` string must be parsed to seconds rather than passed to
`encode-time` as UTC. Published-without-date is omitted so the server stamps it;
drafts carry no publication time even with a date; scheduled-without-date is
also omitted.

## Acceptance

- The eight publication-time `ert-deftest` forms are replaced by one
  table-driven ERT test with eight labelled rows.
- The rows preserve IANA New York summer and winter conversion, numeric `-0500`
  and `-05:00` offsets, and scheduled publication.
- The rows preserve nil outcomes for published-without-date, draft-with-date,
  and scheduled-without-date.
- Every row constructs its entry through `jaunder-test--entry` and compares
  `jaunder-entry-published` with its exact expected value.
- Failure output includes the exact row label.
- Adjacent offset/zone helper tests and live Protocol Client integration tests
  remain separate and unchanged.
- The pure ERT population, Elisp formatting, and warnings-as-errors
  byte-compilation pass; normal CI retains its hermetic Elisp coverage
  authority.

## Boundaries

- Do not change Protocol Client production code, Org metadata interpretation,
  lifecycle rules, timezone conversion, serialization, or live harness behavior.
- Do not add production helpers, shared test abstractions, dependencies, or an
  ADR.
- Do not migrate adjacent offset/zone helper tests or live publish tests.
