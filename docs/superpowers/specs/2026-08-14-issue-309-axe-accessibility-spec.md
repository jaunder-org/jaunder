# #309 — axe accessibility assertions for core CSR states

Issue: [#309](https://github.com/jaunder-org/jaunder/issues/309). Milestone:
Test infrastructure & E2E. The issue has no blockers.

## Problem

The Playwright suite verifies behavior and exact pixels across the four
`{sqlite,postgres}×{chromium,firefox}` combinations, but it does not verify the
accessibility tree and related document semantics. Regressions such as an
unlabelled control, invalid ARIA relationship, missing landmark, or insufficient
contrast can preserve selectors, behavior, and screenshots while remaining
invisible to CI.

Automated analysis is necessarily partial: axe can detect machine-checkable
violations, not replace keyboard testing, manual accessibility review, or
inclusive user testing. The missing automated layer is still valuable because it
converts a stable subset of accessibility requirements into an ordinary merge
gate.

## Decisions

| ID     | Decision                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| ------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **D1** | Add one whole-document axe assertion to each of the four existing `@visual` behavioral tests: public timeline/default theme in `theme.spec.ts`, login form in `auth.spec.ts`, authenticated `/app` cockpit in `authed-flash.spec.ts`, and empty `/posts/new` composer in `posts.spec.ts`. Add `@accessibility` as a second Playwright tag on those tests. Do not create accessibility-only tests or a new Playwright project. The existing visual projects already run these states exactly once per Chromium and Firefox lifecycle before mutation-heavy tests, preserving the ordering and isolation established by #308. |
| **D2** | Enforce the machine-checkable WCAG 2.2 Levels A and AA rules available in axe 4.13.0 through tags `wcag2a`, `wcag2aa`, `wcag21a`, `wcag21aa`, and `wcag22aa`. Axe 4.13.0 has no `wcag22a` tag: WCAG 2.2's new Level-A criteria have no corresponding automated axe rule and remain manual review surfaces. Axe best-practice, experimental, and higher-level rules are outside this contract.                                                                                                                                                                                                                               |
| **D3** | The accepted result is zero violations. Fix every violation in the four selected states at its product-markup or styling source. Do not commit an axe snapshot, baseline, allowlist, selector exclusion, rule disablement, impact filter, or result suppression. A future exception requires an explicit policy change rather than a call-site option.                                                                                                                                                                                                                                                                      |
| **D4** | Add a deep test helper with the sole interface `expectAccessible(page: Page): Promise<void>`. It owns `AxeBuilder`, the fixed conformance tags, whole-document scope, analysis, and the assertion. Its failure output identifies each violated rule, impact, help URL, and affected selectors so a CI failure is actionable. Callers supply only the mounted `Page`; they cannot vary rules or scope and therefore cannot create a second accessibility policy.                                                                                                                                                             |
| **D5** | Call `expectAccessible(page)` only after the existing behavioral readiness assertions have proved the intended CSR state is mounted. Keep the behavioral assertions and visual comparison: axe supplements rather than replaces them. Run axe after the screenshot comparison so script injection cannot affect the visual artifact.                                                                                                                                                                                                                                                                                        |
| **D6** | Pin `@axe-core/playwright` at exactly `4.13.0` as an end-to-end development dependency and commit the generated `end2end/package-lock.json` change. The existing `e2ePackage`/`E2E_TYPES_NODE_MODULES` path remains the single dependency closure for host type checking and Nix-VM execution; update its `npmDepsHash`, not the provisioning architecture.                                                                                                                                                                                                                                                                 |
| **D7** | Extend `CONTRIBUTING.md`'s e2e workflow with the accessibility contract and extension pattern: add `@accessibility` to a stable existing behavioral test, wait for its complete intended state, then call the shared helper. Document the zero-violation and no-exclusion/no-disabled-rule policy, the WCAG target, the current four flows, and the fact that automation does not replace manual assessment.                                                                                                                                                                                                                |

## Interface

`end2end/tests/accessibility.ts` owns the policy:

```ts
export async function expectAccessible(page: Page): Promise<void>;
```

The helper:

1. constructs `AxeBuilder` for the supplied page;
2. selects the five WCAG A/AA tags from D2;
3. analyzes the complete current document;
4. formats violations deterministically enough for readable Playwright output;
5. asserts that the violation list is empty.

There is deliberately no options parameter. Scope, rules, and accepted findings
are policy, not per-test data. If the deletion test were applied, every caller
would otherwise have to reconstruct these decisions; the module earns its seam
by keeping them local.

The four tests retain their existing ownership and state setup. Their metadata
becomes equivalent to:

```ts
{
  tag: ["@visual", "@accessibility"];
}
```

After each test's current readiness assertions and `expectVisual(...)` call:

```ts
await expectAccessible(page);
```

No fixture extension is needed. Axe is requested by four explicit call sites,
not ambient behavior for every test, and the helper does not participate in the
ordering-sensitive auto-fixture chain in `fixtures.ts`.

## Failure and remediation contract

An axe violation fails the owning Playwright test in the same browser/backend
combination that rendered it. The report must expose enough information to find
the rule and DOM targets without re-running merely to discover what failed.

Remediation changes the semantic HTML, ARIA, or styling that caused the finding.
A finding is not fixed by narrowing analysis, disabling its rule, accepting its
impact level, or deleting the assertion. Because the same four tests also carry
exact visual comparisons, any visible remediation updates baselines only through
the existing `cargo xtask e2e-local --update-visual-snapshots` workflow;
invisible semantic fixes require no image churn.

The helper makes no claim that a zero result means the page is fully accessible.
Keyboard sequence, focus order that requires human judgment, screen-reader
quality, cognitive load, and other non-automatable properties remain manual
review surfaces.

## Verification

Implementation is complete when all of the following hold:

1. `devtool run -- devtool check tsc`, from the refreshed Nix development
   environment, provisions `end2end/node_modules` from the updated
   `E2E_TYPES_NODE_MODULES` closure and accepts the helper and all four call
   sites.
2. After one selected flow's screenshot passes, a temporary DOM mutation adds a
   visual-neutral invalid ARIA attribute value before `expectAccessible`. The
   targeted `cargo xtask e2e-local <spec>` run then fails at axe, and its report
   contains the violated rule ID, impact, help URL, and affected selector(s).
   Removing the uncommitted mutation makes the same run pass.
3. The four selected flows pass with zero WCAG 2.2 A/AA violations in the full
   `cargo xtask validate` gate, covering both storage backends and both
   browsers.
4. Existing visual snapshots remain exact, or any product-visible remediation
   has reviewed Chromium and Firefox baseline updates produced only by the
   supported updater.
5. `CONTRIBUTING.md` gives a reader one canonical extension procedure and does
   not imply that automated analysis is complete accessibility coverage.

## Non-goals

- Exhaustive accessibility coverage of every route, dialog, or interaction.
- A dedicated accessibility project, CI lane, reporter, or baseline store.
- WCAG AAA, axe best-practice, or experimental rule enforcement.
- Keyboard-navigation, screen-reader, or other manual-assessment automation.
- Per-test rule configuration, exclusions, disabled rules, impact thresholds, or
  accepted-debt inventory.
- Changes to Playwright project ordering, retry behavior, browser/backend
  matrix, or the visual baseline updater.

## Architecture and domain impact

This extends ADR-0051's single Playwright configuration and preserves ADR-0039's
project ordering and shared-state isolation. It introduces no architectural
choice that is hard to reverse or surprising outside the e2e harness, so no ADR
is warranted. It adds no Jaunder domain concept; `CONTEXT.md` remains unchanged.
