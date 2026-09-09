# Coverage Exception Audit

## Outcome

Every active Rust coverage exception has an evidence-backed disposition, with a
presumption against retention. Practical exceptions are replaced by
consumer-observable coverage or complexity reduction; unavoidable exceptions
remain only at their smallest justified span with a durable source-local reason.

The audit preserves the authoritative host coverage population, SQLite and
PostgreSQL union, line-coverage policy, and CRAP threshold. It reports complete
before/after counts and dispositions in the pull request and issue record.

## Load-bearing decisions

- The reconciled baseline is 95 active line-form `cov:ignore` sites, 63 paired
  `cov:ignore` blocks, and 8 `crap:allow` overrides. Prose and fixture strings
  that merely contain marker text are not active exceptions.
- Each marker is judged from the exact behavior or complexity it exempts and
  whether that behavior is executable in the authoritative host coverage
  environment. Existing integration coverage overrides stale rationales.
- The default disposition is removal. Retention is limited to demonstrated
  non-host, genuinely unreachable, fault-injection-only, generated/build-script,
  or compiler-bookkeeping cases where an honest behavioral test or simplifying
  refactor is impractical.
- Provably dead paths use literal message-bearing `unreachable!` structural
  exemptions in preference to permanent manual markers, preserving ADR-0050's
  self-reflagging invariant.
- Retained line exclusions use `// cov:ignore: <specific reason>`. Retained
  block exclusions start with `// cov:ignore-start: <specific reason>` and end
  with canonical `// cov:ignore-stop`. Empty reasons and legacy bare line/start
  forms are rejected fail-closed.
- The gate continues to reject nested, unmatched, and stray block markers. It
  does not impose an arbitrary line-count cap: semantic breadth is not
  equivalent to formatted span length. This audit removes or narrows every
  currently broad exclusion; future semantic breadth remains a review
  responsibility.
- `crap:allow` keeps its existing non-empty reason and exclusive `T = 30`
  contract. Each override is removed by complexity reduction or meaningful
  coverage when practical, not by weakening the threshold.
- The structural census is closed over four repository-derived classes:
  root-product versus auxiliary-workspace membership; the coverage source
  filter's admitted and excluded Rust trees; crate/module `cfg` gates that keep
  product Rust source out of the host build; and literal message-bearing
  `unreachable!` invocations recognized by the gate. Each rule is dispositioned
  once with the complete current member set derived from its owning manifest,
  Nix expression, module declarations, or exemption parser.
- WASM-only component, client, and CSR code remains outside host coverage and
  under its separate browser/WASM/e2e verification contract; it is not
  force-fitted into host coverage.
- The complete point-in-time disposition table is review evidence in the pull
  request and issue record, not a committed snapshot that can become a stale
  second source of truth. Retained intent lives beside source; durable policy
  lives in the gate, repository guidance, a proposed ADR, and the architecture
  view.

## Acceptance

- A reproducible final census accounts for every baseline line marker, block,
  CRAP override, literal message-bearing `unreachable!` exemption, and member of
  the four defined structural classes; no item lacks one disposition.
- Every removed exception is backed by observable coverage or a simplifying
  refactor, and the authoritative coverage gate passes with no line or CRAP
  threshold reduction.
- Every retained exception is minimal, reason-bearing, and belongs to one of the
  accepted rationale categories; no broad function or module exclusion remains.
- Parser and gate regression checks reject empty-reason and legacy bare
  `cov:ignore` line/start forms while preserving real-comment anchoring,
  balanced-block hard errors, and canonical stops.
- Coverage verification exercises both SQLite and PostgreSQL in the established
  union and leaves the product/auxiliary-workspace and host/WASM boundaries
  unchanged.
- The pull request and issue record contain per-exception dispositions,
  before/after counts by exception kind, remaining counts by rationale category,
  and the complete structural-boundary member inventory.
- Repository coverage guidance, the proposed decision record, and
  `docs/ARCHITECTURE.md` agree with the delivered reason-bearing marker
  contract.

## Boundaries

- No weakening of coverage membership, source filtering, line policy, CRAP
  threshold, backend parity, or fail-closed subprocess/report behavior.
- No committed baseline, generated exception manifest, or point-in-time audit
  snapshot.
- No arbitrary exception-count target, block-size limit, artificial host
  surrogate for browser-only behavior, or broad fault-injection framework added
  solely to reach otherwise impossible lines.
- No redesign of workspace ownership, WASM component boundaries, unrelated test
  infrastructure, or application behavior beyond refactors needed to remove an
  audited exception.
