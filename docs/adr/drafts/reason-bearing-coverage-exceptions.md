# ADR-DRAFT: Reason-bearing Rust coverage exceptions

- Status: proposed
- Date: 2026-09-09
- Issue: [#1417](https://github.com/jaunder-org/jaunder/issues/1417)

## Context

[ADR-0050](../0050-stateless-coverage-gate.md) made `cov:ignore` the stateless
Rust coverage gate's sole manual acceptance path. Its original grammar allowed
bare line and block-start markers. That makes a permanent coverage blind spot
too easy to add without recording why authoritative host coverage cannot
honestly exercise the exact span.

The #1417 audit establishes a presumption against retaining an exception:
consumer-observable coverage or a simplifying refactor is preferred. A retained
exception is limited to a smallest justified span for non-host behavior, a
genuinely unreachable path, impractical fault injection, generated/build-script
code, or compiler bookkeeping. Those categories focus review but never replace
source-local facts.

The audit's complete, point-in-time disposition table belongs in the pull
request and issue record. Committing it would create a stale second source of
truth; durable intent belongs beside each retained source span and durable
policy belongs in this decision and the gate.

## Decision

Require a non-empty, specific reason on every manual Rust coverage exclusion:

- `// cov:ignore: <specific reason>` excludes one line.
- `// cov:ignore-start: <specific reason>` begins an exclusion block.
- `// cov:ignore-stop` ends an exclusion block and takes neither a reason nor
  trailing text.

The parser recognizes markers only in real trailing comments. It rejects legacy
bare line/start forms, empty reasons, noncanonical stops, and nested, unmatched,
or stray blocks as hard errors. No compatibility grammar survives.

No arbitrary block-size limit is imposed. Formatted line count is not a measure
of semantic breadth; reviewers instead assess the exact exempted behavior and
whether its reason remains true. The existing non-empty `crap:allow` grammar and
exclusive CRAP threshold remain unchanged.

## Consequences

Every retained manual exclusion becomes an explicit, durable claim that can be
reviewed with the exact source span. New exclusions carry a modest prose cost,
which is intentional: absence of authoritative host coverage is exceptional, not
a routine annotation.

The gate remains stateless and fail-closed, while its authoritative coverage
population, source membership, SQLite/PostgreSQL union, and line/CRAP thresholds
remain unchanged. The review and issue retain the audit evidence without making
that transient census repository policy.
