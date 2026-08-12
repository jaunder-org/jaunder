# ADR-0122: One bad row must not stop the scan

- Status: accepted
- Date: 2026-08-11

## Context

Several storage reads iterate many rows whose text columns pass through
validating newtype decodes (`FeedPath`, `Filename`, hash columns). A row written
under an older grammar, hand-edited, or corrupted can fail that decode. If the
whole read returns `Err`, one bad row takes down the entire surface: a user's
media list 500s, or — worse — the feed worker never advances `last_tick` past an
error, so a failing catch-up scan retries forever and go-live enqueueing never
resumes.

## Decision

Scans and lists skip (or, for the feed-event claim, divert to a purge list) a
row whose _validated text column_ fails to decode, and process the rest. Three
guardrails:

- **Direct lookups stay strict.** `get_media`/`find_by_hash` and other
  single-row reads surface the error — a targeted read of a bad row is a real
  failure.
- **The diversion is column-scoped.** A wrapper that treated _any_ decode
  failure as "corrupt" would widen a destructive path (the feed-event purge
  DELETEs) from one column to ten; a schema change or driver regression would
  silently drain the queue. Only the column the policy names may divert;
  anything else propagates (#728).
- **Identity failures propagate.** If the row's own id will not decode there is
  no third state: the batch fails.

## Consequences

- One unusable row costs only itself, on every bulk surface.
- Each site carries a one-line pointer here instead of restating the argument;
  the dual-backend tests assert the skip/purge behavior per site.
