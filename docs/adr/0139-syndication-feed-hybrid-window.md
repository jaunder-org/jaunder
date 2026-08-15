# ADR-0139: Select Syndication Feed items with a hybrid window

- Status: accepted
- Date: 2026-08-14
- Issue: [#937](https://github.com/jaunder-org/jaunder/issues/937)

## Context

A count-only feed gives quiet publications useful history but can omit much of a
busy publication's recent interval. A time-only feed preserves that interval but
can leave quiet feeds nearly empty. Jaunder ships `HybridWindow`, configured by
`feeds.min_items` and `feeds.min_days`, without a decision record explaining the
union, defaults, time boundary, visibility ordering, or cache activation.

Syndication Feeds are public projections, so anonymous visibility from
[ADR-0020](0020-content-visibility-and-subscription-model.md) must be resolved
before membership has meaning. They are cached materializations whose scheduled
publication transitions are already made durable by
[ADR-0027](0027-scheduled-publishing-time-gated-visibility.md).

## Decision

For eligible Posts ordered by `published_at DESC, post_id DESC`, a Syndication
Feed contains the union of:

- the first `feeds.min_items` Posts; and
- every Post whose publication time is at least the inclusive `feeds.min_days`
  cutoff.

Anonymous/Public eligibility is applied before ranking and selection. The
defaults are 20 Posts and 30 fixed 24-hour UTC days. The count floor serves
quiet publications; the age interval serves busy publications. Count-only,
time-only, an intersection, and a hard maximum are rejected because each drops
one of those guarantees.

Membership is exact when the feed is regenerated. Time passing alone does not
schedule age-out work, so a cached feed may retain an older Post until another
regeneration. This is an explicit regeneration-snapshot contract, not a claim
that the 30-day boundary is continuously materialized.

A successful valid setting mutation durably invalidates every cached Syndication
Feed before returning. Unset settings use the 20/30 defaults. Cutoff arithmetic
is checked. A day value too large for date arithmetic selects all history rather
than panicking. Corrupt stored values surface a configuration error rather than
silently using defaults. This narrowly supersedes
[ADR-0102](0102-config-key-closed-registry.md)'s unchanged defensive-read
behavior for `feeds.min_items` and `feeds.min_days`; other keys are unchanged.
No arbitrary maximum is imposed without an operationally justified bound. Typed
configuration remains governed by
[ADR-0063](0063-domain-value-newtype-convention.md).

## Consequences

Feed size is not strictly bounded: a busy publication can have more than the
count floor within the age interval, and an extreme valid age can select all
history. That cost is the direct consequence of preserving both guarantees.

The regeneration-snapshot rule avoids a new per-feed timer or periodic sweep.
Readers may see an item older than the configured interval until the next feed
event; the cache is never described as continuously age-exact.

Current production behavior already implements the union, defaults, inclusive
fixed-duration cutoff, and deterministic prefix selection.
[SQL ranks before anonymous/Public eligibility](https://github.com/jaunder-org/jaunder/issues/1051),
so private rows can crowd out the count floor.
[Setting activation, arithmetic, and corrupted reads](https://github.com/jaunder-org/jaunder/issues/1053)
remain implementation debt.
