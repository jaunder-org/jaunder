# Approved: raise PostgreSQL/Chromium suite cap (#1827)

## Outcome

Increase only the PostgreSQL/Chromium Playwright suite cap from 1,500 to 1,800
seconds. Preserve the existing 300-second lifecycle allowance and 120-second
deadline reserve, so its VM global deadline becomes 2,220 seconds. This is added
headroom, not a root-cause diagnosis or assurance that the suite will pass.

## Approach and tradeoffs

At `nix/checks.nix`, the current shared cap also serves other backend/browser
lanes and the optional Firefox census preflight. Raising that constant to 1,800
would expand budgets across those paths. Use a scoped override through the
existing E2E check/combination parameter boundary, without extra machinery; keep
other lanes and preflight unchanged. A modestly larger PostgreSQL/Chromium VM
allowance is accepted. CI workflow source has no explicit job timeout; the
xtask/Nix execution path's VM global deadline is the relevant explicit source
deadline found. No vendor default is relied upon.

A passing PostgreSQL/Chromium run used 1,487,702 ms of its 1,500,000 ms cap
(12,298 ms remaining); it matched the checked 490-result population. This
supports adding more headroom but is not causal proof, a guarantee, or evidence
that this change cures the regression.

## Acceptance

Extend the existing Nix primitive budget evaluation test seam to prove the
PostgreSQL/Chromium override is 1,800 seconds, its outer deadline is 2,220
seconds, inner timeout remains below outer deadline, and other lanes retain
current values. Exercise actual lane selection and parameter wiring, not merely
a duplicated formula. This existing seam is confirmed testing authority.
Preserve per-test timeout budgets and the ≥80% duration-pressure rejection
guard, workers, resources, retries, population, and assertions. Acceptance
requires authoritative PostgreSQL/Chromium CI through the normal PR/CI
lifecycle.

## Boundaries and approval

Approval `1827-suite-cap-v1`: Manager relays the direct User answer, “#1827 is
approved. Why haven't you been pushing forward these other issues?” This
approves the exact PostgreSQL/Chromium 1,800-second suite cap and 2,220-second
VM deadline implementation and normal PR/CI lifecycle, not merge. Per-PR merge
approval remains required. No other lane, Firefox census preflight, per-test
budget, duration guard, workers, resources, retries, population, assertions, or
presentation changes are authorized. Do not claim that this resolves the
failures.

No outline is required: this is narrow budget parameter wiring through an
existing boundary and testing seam, with no new architectural boundary. This
working spec is maintained through implementation and conformance review, then
deleted under the contributor artifact lifecycle.
