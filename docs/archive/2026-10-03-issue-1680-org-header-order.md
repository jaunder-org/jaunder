# Issue #1680 — Stable local Org metadata order

## Outcome

A Post managed by the Emacs Protocol Client keeps its locally synthesized
`JAUNDER_*` header properties in one deterministic order after successful pull
or server-confirmed publish/write-back. Unchanged values no longer move around
or produce unrelated Git diff lines just because a later operation added another
property.

## Load-bearing decisions

- Use the pull renderer's established property order as the canonical order:
  `JAUNDER_STATUS`, repeated `JAUNDER_AUDIENCE` (in their existing canonical
  order), `JAUNDER_DATE_TZ`, `JAUNDER_DATE_UTC`, `JAUNDER_FORMAT`,
  `JAUNDER_SLUG`, `JAUNDER_ID`, `JAUNDER_SYNCED`, then `JAUNDER_SYNCED_AT`.
  Still-active recovery properties follow these stable fields in this order:
  `JAUNDER_LOCAL_AHEAD`, `JAUNDER_CREATE_KEY`, `JAUNDER_CREATE_DIGEST`,
  `JAUNDER_CREATE_ATTEMPT_AT`.
- Reorder only recognized client-managed `JAUNDER_*` properties in the leading
  Org header. Keep author-owned keywords, unknown directives and Post body
  unchanged, including the authored headers' relative order. Do not manufacture
  absent fields or change their values merely to order them.
- A successful pull already synthesizes a fixed header; successful write-back
  must converge to the same order for the shared fields. Repeated audiences
  remain in their canonical semantic order rather than being collapsed into a
  single property.
- Do not cosmetically reorder on a failed request or before the server-confirmed
  write-back. Preserve the create identity checkpoint, retry markers and
  subsequent cleanup saves required by ADR-0047; normalization must not
  undermine recoverability if interrupted between those saves.
- This is a local presentation/serialization correction, not a change to the
  server's canonical metadata-free Org body (ADR-0024, ADR-0155) or to protocol
  authority.

## Acceptance

- Given an existing local Post with interleaved or out-of-order recognized
  Jaunder properties, a successful server-confirmed update produces the
  canonical property sequence, preserving unchanged IDs/timezones and authored
  content; another successful update leaves the order stable.
- A successful create and a fresh pull produce the same relative order for their
  common properties, including publication time, slug, ID and synchronization
  fields. Repeated audiences remain present and correctly ordered.
- Failure before server confirmation does not reorder pre-existing header lines.
  Existing pre-send metadata mutations, including timezone capture and durable
  create-intent markers, remain permitted. Create/replay checkpoint tests still
  show a durable ID and valid sync baseline before intent markers are cleared;
  any surviving recovery marker is deterministically placed on success.
- Pure ERT and applicable live Emacs-client integration tests cover the ordering
  and unchanged round-trip semantics. The existing Emacs formatting, ERT,
  byte-compilation and coverage gates remain green.

## Boundaries

- No wholesale sort of authored Org keywords or body, no rewrite of unknown
  `JAUNDER_*` extensions, and no server-side or AtomPub representation change.
- No migration that rewrites files independently of a successful
  pull/write-back. Existing files converge when next successfully synchronized.
- No new ADR: this applies ADR-0024's fixed local synthesis principle and
  ADR-0047's retry-safe write-back ordering rather than changing either
  architectural decision.
