# Issue #817: E2E backend performance interpretation

## Outcome

Jaunder's observability guidance records what the preserved #792 corpus shows:
the current-gate-equivalent arm-B E2E suite medians differ by no more than
±1.5%, while SQLite storage operations cost about 1.4× PostgreSQL. Future
performance work can choose one backend for suite-level questions without
inheriting the false claim that the backends themselves perform alike.

## Load-bearing decisions

- The 24 preserved captures from #792 remain the evidence: six runs across all
  four backend/browser combinations at gate-identical settings.
- Suite wall-clock and server/storage span cost are separate measurement frames.
  The current-gate arm-B medians are backend-independent within the suite noise
  floor; server work is not.
- The explanation is dilution, not equivalence: server time is about 18% of
  in-test span time, so SQLite's roughly 20-second-per-combo server delta
  becomes roughly 10 seconds of wall-clock under two workers inside a 174–258
  second client-dominated suite.
- Current guidance carries the qualified claim and its shelf life: reducing
  client cost increases the server share, so future work must reconsider the
  backend axis when that composition changes.
- Historical #792 measurements and methodology remain historical evidence. Only
  unqualified shorthand such as `sqlite ≈ postgres` is corrected to state that
  it refers to suite wall-clock.

## Acceptance

- `docs/observability.md` states the current-gate arm-B median result within
  ±1.5%, the approximately 1.4× SQLite storage-operation cost, and why both
  statements are simultaneously true.
- The recorded comparison includes per-browser SQLite/PostgreSQL suite medians
  and server-side distributions (`request` totals/p90 and `storage.*`
  totals/p50), showing that the result is consistent across Chromium and Firefox
  rather than hidden by one aggregate.
- The explanation names the roughly 18% server share that dilutes the backend
  delta and the shelf-life trigger: if client-side cost falls enough to increase
  that share, performance work must restore the backend axis.
- Guidance says when one backend is sufficient and when server/storage analysis
  must retain the backend axis.
- The #792 historical section's shorthand is explicitly limited to suite
  wall-clock without changing its tables, measurements, or reproduction notes.
- Any live spec that cites #792 as backend-equivalence evidence is similarly
  qualified as suite-level rather than server/storage equivalence.
- No unqualified claim that SQLite and PostgreSQL are statistically identical
  remains in current performance guidance.

## Boundaries

- No new traces are collected and the preserved corpus is not regenerated.
- Why SQLite storage operations are slower is not investigated.
- No E2E gate, threshold, analyzer implementation, backend configuration, or
  runtime behavior changes.
- Semantic backend parity, the four-combination CI matrix, and historical #792
  warmup conclusions remain unchanged.
