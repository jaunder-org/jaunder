# ADR-DRAFT: PR observation is a native `dev pr-watch` with membership-based early failure

- Status: proposed
- Date: 2026-11-24
- Issue: [#2067](https://github.com/jaunder-org/jaunder/issues/2067)

## Context

[ADR-0261](../0261-tool-language-by-maintainability.md) moved `pr-observer` to
Python after a feature audit. The Rust tool was about 5,300 production lines.
Its only machine consumer, `scripts/pr-land.sh`, reads `pr`, `head_sha`,
`outcome`, `pointer` and `detail` and the 0/1/>1 exit codes.

About 2,400 lines reimplemented GitHub Actions' job graph
([ADR-0195](../0195-dynamic-pr-check-classification.md),
[ADR-0198](../0198-active-workflow-incomplete-pr-evidence.md)): matrix
expansion, reusable-workflow inlining, and re-running `ci.yml`'s `jq` catalog
command after string-matching it. Every job in `ci.yml` feeds one of the two
`always()` aggregates the ruleset requires, so the graph only made a failed
upstream job (typically an e2e lane) terminal before its aggregate settled.
About 860 more lines matched failure log signatures across PRs (#1499); no
consumer read that field and it never changed a verdict.

## Decision

- PR observation is a module of the Python `dev` package that implements
  `dev pr-watch` in-process. It is read-only: it never arms, merges,
  re-enqueues, cleans up or edits the tracker. `scripts/pr-land.sh` remains the
  only armer and uses this same observer. This amends
  [ADR-0240](../0240-just-frontdoors-host-pr-observer.md)'s `tools/pr-observer`
  host executable and removes `pr-observer` from
  [ADR-0258](../0258-dev-runs-nix-built-host-tools.md)'s Nix-built host tools.
- The CLI, outcomes, exit codes, report fields and stderr progress keep their
  contract, except the dropped `shared_failure` field and
  `awaiting-classification-evidence` phase.
- Early failure uses workflow membership, superseding the graph classification
  mechanism of ADR-0195 and ADR-0198. A failed GitHub Actions check that is not
  itself a ruleset context is required when its workflow file, read at the PR
  head, defines a job whose literal name (or key, when unnamed) is a ruleset
  context. Otherwise, including when the file is unreadable, it is an optional
  warning. Misjudging toward optional only delays `checks-failed` until the
  aggregate fails. No graph, `needs`, matrix or expression evaluation; no
  hard-coded check, job or workflow names. Approved by the human at spec
  approval for #2067, as ADR-0240 requires for changes to these guarantees.
- These ADR-0195 rules still hold: readiness requires every ruleset context to
  have succeeded; checks on a superseded head never affect the verdict; a
  pending re-run outranks its earlier failure and otherwise the latest
  completion wins; a non-Actions status context is required only by exact
  ruleset membership.
- Rate limits are transient poll errors under the strike budget; there is no
  reset lookup. Shared-failure enrichment is removed.

## Consequences

- One observer, no Nix build step for `pr-watch`, and `dev`'s lint, type and
  100% branch-coverage gates apply to it. PyYAML joins the `dev` closure.
- A failed optional job placed inside a workflow that also defines a required
  context ends the watch as `checks-failed`. This errs toward blocking; move
  such a job to its own workflow. A required context named by an expression, a
  matrix or a reusable workflow never matches, so its upstream failures surface
  only when the aggregate fails; do not reintroduce graph evaluation for that
  latency.
- A rate-limit exhaustion longer than the strike budget ends as `watcher-error`
  rather than sleeping to the reset.
- Agents diagnosing a possibly shared CI failure check main's status or the
  linked log themselves.
