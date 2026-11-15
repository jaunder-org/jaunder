# Issue #1954 — reconcile cache-policy.json with the split E2E lanes

## Outcome

`just source-diagnostics cache-safety-probe` passes again on `main`, and when
the checked policy next drifts from the Nix inventory the failure names the
attributes that differ.

## Approach and tradeoffs

- **Policy data.** In `nix/cache-policy.json`, replace the four retired
  `checks.x86_64-linux.e2e-{sqlite,postgres}-{chromium,firefox}` finals and
  their four `-driver` supports with the names the generated inventory
  (`packages.x86_64-linux.cache-safety-inventory`) now emits. Each split
  producer keeps its predecessor lane's class; the new workers controls are
  final, as the inventory classifies them:
  - finals: `checks.…-e2e-{db}-chromium-unsplit`,
    `checks.…-e2e-{db}-firefox-ordinary-{1,2}-of-2`,
    `checks.…-e2e-{db}-firefox-serial-special`, and the controls
    `packages.…-e2e-{db}-firefox-workers-{2,4}-control`;
  - supports (`sourceFamily: e2e-driver`): the `-driver` package of each of
    those twelve.

  Unchanged entries (`*-single-worker*`, `e2e`, `e2e-checks`, `e2e-support`,
  `e2e-npm-deps`, coverage) stay as they are.

- **Diagnostic.** The classification-mismatch error in the probe's reconcile
  step lists, per class, the attrs the policy is missing and the attrs it has
  extra, instead of only saying they differ. A unit test pins that message.
- The checked policy stays a hand-reviewed second key against the generated
  inventory; generating it would remove the review it exists to force, and is
  out of scope here.

## Acceptance

- `traced-run -- just source-diagnostics cache-safety-probe` exits 0 on the
  fixed tree. This covers the later stages too (push filter, non-substitutable
  metadata, closure leak and salt mutation), not just reconcile. It runs longer
  than before: the `e2e-driver` source probes go from 8 drivers to 16.
- The probe's existing unit tests pass, plus one asserting the mismatch error
  names a missing and an extra attr. That test uses non-empty, disjoint finals
  and supports so it gets past the earlier malformed-inventory check.

## Boundaries

- No change to Nix lane definitions, the inventory derivation, source families,
  or when the probe runs (ADR-0227 on-request status stands).
- No other reconcile messages are reworded.
