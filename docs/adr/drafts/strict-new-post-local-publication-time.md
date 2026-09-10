# ADR-DRAFT: New-Post publication time rejects nonexistent local times

- Status: proposed
- Date: 2026-09-10
- Issue: [#1441](https://github.com/jaunder-org/jaunder/issues/1441)

## Context

The full new-Post composer previously used one native `datetime-local` input.
That control can display a partially edited date while exposing an empty value
to JavaScript, which let the Publish action mistake an incomplete schedule for
“publish now.” [ADR-0182](../0182-jiff-time-model.md) retained browser-style gap
normalization for that HTML datetime-local seam.

The replacement creation control exposes separate Date and Time fields behind an
explicit Apply step. Normalizing a nonexistent wall time during Apply would
silently commit an instant different from the value the author reviewed. The new
explicit commit boundary can instead report the invalid choice before any Post
is created.

## Decision

The full `/posts/new` publication-time control converts its provisional local
Date and Time strictly when Apply commits them. A nonexistent local time in a
daylight-saving gap is rejected inline and leaves the committed publication time
unchanged. An ambiguous fall-back time continues to choose the earlier instant.

This supersedes ADR-0182 only for the full new-Post creation control. The draft
publish-at `datetime-local` seam retains browser-normalizing gap conversion. The
Scheduled Post editor already uses strict conversion and remains unchanged by
this issue. `UtcInstant` remains the absolute-time boundary, and the browser
continues to perform local-to-UTC conversion with bundled IANA time-zone data.

## Consequences

- Apply never silently shifts the author-reviewed creation time across a
  daylight-saving gap.
- An author choosing a nonexistent time must enter a valid local Date and Time
  before scheduling the Post.
- The draft publish-at control keeps browser-normalizing conversion, and the
  Scheduled Post editor keeps its existing strict conversion; this issue does
  not change either edit lifecycle.
- The decision changes no storage schema, Scheduled Post derivation, public
  visibility gate, or Syndication Feed behavior.
