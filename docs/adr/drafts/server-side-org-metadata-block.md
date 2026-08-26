# ADR-DRAFT: Server-Side Org Metadata Block Canonicalization

- Status: proposed
- Date: 2026-08-26
- Issue: [#77](https://github.com/jaunder-org/jaunder/issues/77)

## Context

[ADR-0024](../0024-server-side-org-canonicalization.md) established that every
Org write reaches one metadata-free stored body, but deliberately deferred
full-header parsing. That limit leaves a raw-Org create or update unable to
express the same structured post metadata as other authoring surfaces, and would
make each surface responsible for a different interpretation of the same source.

The server must accept Org as a first-class authoring representation without
letting metadata live independently in both the header and structured post
fields. It must also distinguish recognized Jaunder metadata from arbitrary Org
directives, preserve the latter as author content, and reject an invalid write
before it can partially alter either metadata or body.

## Decision

This ADR evolves ADR-0024: on every Org create and update, the server parses the
complete leading Org/Jaunder metadata block, case-insensitively. The block ends
immediately before the first top-level Org element that is not a keyword. After
the whole write is accepted, every recognized header is removed before body
canonicalization, including valid mutable metadata displaced by structured
input; unrecognized Org directives remain in the canonical body. Consequently,
stored Org bodies contain no recognized mutable metadata or bookkeeping and
every creation/update surface converges on the same structured post plus
canonical metadata-free source.

Recognized mutable metadata is `#+TITLE`, repeated or comma-separated
`#+KEYWORDS`, repeated `#+DESCRIPTION`, `#+DATE`, and `#+PROPERTY` values for
`JAUNDER_DATE_TZ`, `JAUNDER_STATUS`, and repeated `JAUNDER_AUDIENCE`. Text and
list values compose by field; date, lifecycle, timezone, and bookkeeping values
are singletons. Values pass through the existing typed title, summary, tag,
slug, ID, and timestamp boundaries. Blank recognized values reject except that
each `KEYWORDS` occurrence drops empty comma terms and must retain at least one
valid term. Audience values are exactly `public`, `subscribers`, `private`, or
`named:<numeric-id>`; `private` cannot combine, and named IDs must belong to the
author.

Structured presence is resolved per field. A supplied valid scalar or
collection, including an empty collection, wins; otherwise the header may fill
the field. When both omit it, the surface keeps its existing update omission or
create default semantics. Lifecycle status and publication time merge as one
unit from one source, never as independently selected fields; transport defaults
do not manufacture presence.

`DATE` is Emacs's inactive `[YYYY-MM-DD Ddd HH:MM]` form with a matching weekday
and required IANA `JAUNDER_DATE_TZ`. Ambiguous DST folds choose the earlier
instant; nonexistent local times reject. One request clock classifies the
result, with equality non-future. A header lifecycle always includes
case-insensitive `JAUNDER_STATUS`: `draft` permits neither date nor timezone;
`scheduled` requires both and a future instant; `published` permits neither and
uses the request clock, or requires both and a non-future instant. Date and
timezone never appear independently.

`JAUNDER_FORMAT`, `JAUNDER_SLUG`, `JAUNDER_ID`, `JAUNDER_SYNCED`,
`JAUNDER_SYNCED_AT`, and `JAUNDER_DATE_UTC` are singleton `#+PROPERTY`
bookkeeping, not input authority. Create rejects ID/sync fields; slug, format,
and publication UTC must match the final stored representation. Update requires
ID to match the target, slug/format/publication UTC to match final effective
stored values, and `JAUNDER_SYNCED` to match the current pre-write content ETag;
`JAUNDER_SYNCED_AT` is syntax-only. Times compare as RFC 3339 instants.

Parsing, merging, authorization, derived-field checks, and body stripping form
one atomic acceptance decision. Malformed or conflicting metadata, a foreign or
invalid audience, and metadata-only content reject without stripping or saving.
Web uses Validation except Conflict for stale sync; AtomPub uses 400 except 412
for stale sync, without revealing whether a foreign audience exists.

## Consequences

- Good: raw Org gains the full structured metadata vocabulary on every ingress
  path, while headers cannot silently outrank explicit structured requests.
- Good: the server owns one deterministic interpretation and stored body form;
  clients can synthesize presentation headers without preserving duplicate
  authoritative state.
- Good: unknown directives remain round-trippable author content, while
  validation prevents stale synchronization bookkeeping and foreign audience
  references from becoming persisted state.
- Cost: the server now owns a strict, format-aware parser and a field-specific
  duplicate and precedence policy rather than treating Org headers as opaque
  text.
- Cost: clients must provide internally consistent bookkeeping when they choose
  to send it; malformed metadata rejects the entire write instead of degrading
  to a partially applied post.
- Ruled out: parsing only `TITLE`, treating headers as an alternate
  authoritative post representation, accepting a partial metadata block, or
  stripping headers before all validation succeeds.
