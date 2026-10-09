# Issue #1699: reconciliation fetch performance diagnosis

## Confirmed finding

The reporter's timestamped production trace for Post ID 800 isolates the
slowdown to Local Post Link replacement construction:

- `jaunder--pull-stage-member`: 451.434 seconds.
- `jaunder--pulled-post-link-replacements`: 450.616 seconds.
- `jaunder--reverse-pulled-post-links`: 450.656 seconds, including the above.
- `jaunder--pull-media-materialize`: 0.397 seconds.
- Fresh Collection verification: 9.302 seconds over 38 pages.
- Final report Collection refresh: 9.218 seconds over 38 pages.

At the diagnosed revision, `elisp/jaunder-post-link.el` built replacement proof
by iterating every Member with a usable canonical alternate href against every
local Post. The target predicate checked regular-file status, canonical paths,
and root containment before rejecting mismatched Post IDs. This produced
quadratic filesystem work even when an Org body contained only an image URL,
rather than a Local Post Link needing reversal.

## Local reproduction

The disposable `.xtask/post-link-scale.el` fixture invokes the real reverse
mapping path with actual temporary Org files, matching Member/local Post IDs,
public canonical alternate hrefs, and one unrelated image URL. No HTTP or Media
acquisition is involved. The body remains unchanged.

Observed local measurements:

| Members | Local Posts | Target validations | Elapsed |
| ------- | ----------- | ------------------ | ------- |
| 100     | 100         | 10,000             | 3.398s  |
| 300     | 300         | 90,000             | 27.359s |

A smaller deterministic diagnosis assertion rejects more than ten target
validations for ten Members/local Posts. Invocation:

```sh
devtool run -- emacs --batch -Q -L elisp -l .xtask/post-link-scale.el
```

It exited 255 on the diagnosed implementation, observing 100 validations. This
is a disposable diagnosis fixture, not a committed regression or an approved
implementation contract. Timing observations are not portable performance
budgets; the work-count signal is deterministic.

## Repair proof

With identity-indexed candidate selection, the same real-filesystem probe at
1,000 Members and 1,000 local Posts performs 1,000 target validations in 0.372s.
The small deterministic diagnosis assertion passes. The permanent regression
also proves valid Post-link localization at both 100 and 1,000 Posts, and
compatibility tests retain ambiguous, stale, missing and out-of-root source
links. Twelve focused Post-link and selected-pull tests pass, including the flow
fixture upgraded with public canonical alternate hrefs. Production timing
confirmation remains separate from these local measurements.

## Why the earlier fixture missed it

The selected-pull fixture used 100 remote Members, three local Posts, and three
selected pulls. Its Collection Entries carried edit links, slugs, and ETags, but
no canonical alternate hrefs. Therefore it skipped the expensive
replacement-evidence branch. The expanded 1,000-local/1,000-remote measurement
also lacked those hrefs and did not establish performance for this branch. A
permanent regression must include realistic public permalink metadata.

## Other costs and safety boundary

Repeated complete Collection enumeration is independently demonstrated. The
1,000-Post fixture with 25 Members per page makes 160 Collection reads for three
pulls and 440 for ten, including final refresh. Production captures 38 pages per
enumeration, taking roughly nine seconds. This is a secondary batch cost, not
the measured seven-minute staging bottleneck.

ADR-0200 requires fresh unique identity, reviewed remote strong ETag, unchanged
local bytes and identity, clean visiting buffers, and safe recoverable local
replacement. ADR-0201 requires exact canonical-href and local ID/slug/filename
agreement and preservation of unproven links. Optimizing the join must not
weaken those contracts or turn duplicate evidence into an arbitrary winner.
Sharing freshness evidence across a batch is a separate policy decision, not an
incidental consequence of optimizing Local Post Link reversal.

## Diagnostics scope agreed during diagnosis

The user requested permanent, off-by-default timestamped diagnostics throughout
the Emacs Protocol Client, in a dedicated `*Jaunder Debug*` buffer. The
provisional design covers operation boundaries, elapsed times, failures and
cancellation without logging credentials or authored content. Buffer lifetime,
retention bounds, and final acceptance remain to be recorded in the spec.

Temporary instrumentation is in `.xtask/jaunder-debug-timings.el`; loading it
replaces its installed probes and `jaunder-debug-timings-stop` removes them.
Neither disposable file is intended to ship as the permanent diagnostics API.
