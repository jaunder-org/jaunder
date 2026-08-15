# Axe Accessibility Assertions — Implementation Plan

> **For agentic workers:** Execute with `jaunder-iterate`; use
> `jaunder-dispatch` only for a self-contained task where delegation helps. Tick
> each checkbox when its step is complete.

**Goal:** Gate four core CSR states on zero machine-checkable WCAG 2.2 A/AA
violations with one reusable, actionable Playwright assertion.

**Architecture:** A small TypeScript module owns axe configuration, analysis,
diagnostic formatting, and the zero-violation assertion. The four existing
visual tests call it after their exact screenshot, so the current fresh-state
Chromium/Firefox project ordering supplies accessibility coverage without a new
project or browser lifecycle. Product fixes stay in the CSR module that owns the
invalid markup or styling.

**Tech stack:** TypeScript, Playwright 1.58.2, `@axe-core/playwright` 4.13.0,
Leptos CSR, npm lockfile, Nix `buildNpmPackage`.

**Issue:** [#309](https://github.com/jaunder-org/jaunder/issues/309)

**Approved spec:**
[`2026-08-14-issue-309-axe-accessibility-spec.md`](2026-08-14-issue-309-axe-accessibility-spec.md)

## Review

**Scope in:** Pin axe; add one fixed-policy helper; scan the public timeline,
login, authenticated cockpit, and empty Post composer; repair all findings in
those states to zero; document how to extend the pattern.

**Scope out:** New Playwright projects or CI lanes, other routes, WCAG AAA,
best-practice/experimental rules, baselines, allowlists, exclusions, disabled
rules, impact filtering, keyboard or screen-reader automation.

**Tasks:**

1. Add the axe dependency, assertion module, four core-flow calls,
   zero-violation remediation, documentation, and full verification as one
   atomic test-harness contract.

**Key risks and decisions:**

- Axe 4.13.0 has five applicable WCAG tags; no `wcag22a` tag exists. The missing
  automatable WCAG 2.2 Level-A coverage remains manual.
- The Nix npm dependency hash and the provisioned TypeScript closure must move
  with `package-lock.json`; a locally available package is not proof.
- Axe runs after `expectVisual`, preventing its injected script from affecting
  exact images. Any visible product remediation still requires reviewed browser
  baselines through the existing updater.
- Current violations are discovered only by executing the four real states.
  Remediation files are therefore conditional, but the accepted result is not:
  every finding is fixed at its owning CSR source; none is suppressed.

## Global constraints

- Pin `@axe-core/playwright` at exactly `4.13.0`.
- Use exactly `wcag2a`, `wcag2aa`, `wcag21a`, `wcag21aa`, and `wcag22aa`.
- The only public interface is `expectAccessible(page: Page): Promise<void>`;
  expose no policy options.
- Analyze the whole mounted document and require zero violations.
- No snapshots, baselines, allowlists, selector exclusions, disabled rules,
  impact filters, or result suppression.
- Preserve ADR-0051's single Playwright config and ADR-0039/#308 project order.
  Do not modify `end2end/playwright.config.ts`.
- Exactly the four existing `@visual` tests also carry `@accessibility`; do not
  create accessibility-only tests.
- Keep every existing readiness assertion and screenshot. Call axe after the
  screenshot and before any later state mutation in the owning test.
- Automation supplements rather than replaces manual accessibility assessment.
- Run commands through `devtool run --`. Invoke pinned tools directly; never use
  `npx`, npm scripts, package-manager execution wrappers, or `nix develop -c`.
- Before every kept commit, run `devtool run -- cargo xtask check`, stage the
  exact intended tree, and commit without a `Co-Authored-By` trailer.

---

## Task 1: Add and enforce the four-state accessibility contract

**Files:**

- Create: `end2end/tests/accessibility.ts`
- Modify: `end2end/package.json`
- Modify: `end2end/package-lock.json`
- Modify: `flake.nix`
- Modify: `end2end/tests/auth.spec.ts`
- Modify: `end2end/tests/theme.spec.ts`
- Modify: `end2end/tests/authed-flash.spec.ts`
- Modify: `end2end/tests/posts.spec.ts`
- Modify: `CONTRIBUTING.md`
- Modify if and only if axe reports an owned violation:
  - `web/src/auth/component.rs`
  - `web/src/forms/component.rs`
  - `web/src/forms/field.rs`
  - `web/src/topbar/component.rs`
  - `web/src/topbar/markup.rs`
  - `web/src/sidebar/component.rs`
  - `web/src/sidebar/markup.rs`
  - `web/src/cockpit/component.rs`
  - `web/src/timeline/component.rs`
  - `web/src/posts/component.rs`
  - the exact stylesheet that owns a reported contrast violation
- Modify only if a visible remediation changes pixels: the affected existing
  files under `end2end/tests/*.spec.ts-snapshots/`

**Interfaces:**

- Produces: `export async function expectAccessible(page: Page): Promise<void>`
  in `end2end/tests/accessibility.ts`.
- The helper constructs `new AxeBuilder({ page })`, calls
  `.withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"])`,
  analyzes the whole document, and asserts `results.violations` equals `[]`.
- A private formatter consumes axe-core's `Result[]`, sorts violations by rule
  ID and nodes by serialized target, and emits for every finding: rule ID,
  impact (including `null` explicitly), help text and URL, then each affected
  target selector. This text is the assertion message.
- Each selected test metadata becomes `{ tag: ["@visual", "@accessibility"] }`
  and imports the helper directly. No fixture or config interface changes.

- [x] **Step 1: Pin the npm dependency and refresh the Nix closure hash**

Run:

```bash
devtool run -- npm install --package-lock-only --ignore-scripts --save-dev --save-exact --prefix end2end @axe-core/playwright@4.13.0
```

Inspect `end2end/package.json` and `end2end/package-lock.json`: the direct
version is exactly `4.13.0`, the lock includes its `axe-core` dependency, and no
unrelated dependency changed.

Set `e2ePackage.npmDepsHash` temporarily to `pkgs.lib.fakeHash`, then run:

```bash
devtool run -- nix build --no-link -L --accept-flake-config .#checks.x86_64-linux.static-checks
```

Expected: failure naming the computed `got: sha256-…` hash. Read that value from
the parked stderr, replace the temporary fake hash with it, and rerun the same
command. Expected: PASS, proving the lockfile, Nix fetch closure, node-module
provisioner, and TypeScript dependency resolve together. Never commit the fake
hash.

The changed `e2ePackage` has a new immutable store path, while the current shell
still exports the old path. Before any later host typecheck, gate, or commit,
refresh the repository development environment by leaving and re-entering it.
Confirm `E2E_TYPES_NODE_MODULES` differs from its pre-change value and names the
new `jaunder-e2e/node_modules` store path. Do not use `nix develop -c`.

An agent session whose parent environment cannot be re-entered must run
`devtool run -- nix derivation show .#checks.x86_64-linux.static-checks`, read
the single `.[] | .env.E2E_TYPES_NODE_MODULES` value from the parked JSON, and
pass that exact value through the Bash tool's `env.E2E_TYPES_NODE_MODULES` field
on every subsequent typecheck, `cargo xtask check`, `git commit`, and
`cargo xtask validate` invocation. Assert that the derivation JSON produced
exactly one value. This is the same freshly evaluated closure the static-check
derivation uses, without a second dependency path.

- [x] **Step 2: Implement the fixed-policy axe helper**

Create `end2end/tests/accessibility.ts` with the interface above. Use the
repository's `expect` from `@playwright/test`; do not expose `AxeBuilder`, tags,
scope, formatter, or options to callers. Build the message from a sorted copy of
the violations/nodes so diagnostics are stable without mutating axe's result.
The assertion must compare the original `results.violations` with `[]` and
attach the formatted message.

With the refreshed `E2E_TYPES_NODE_MODULES` environment from Step 1, run:

```bash
devtool run -- devtool check tsc
```

Expected: PASS after `devtool check tsc` provisions `end2end/node_modules` from
the updated closure.

- [x] **Step 3: Prove the helper rejects an actionable semantic defect**

First add `@accessibility` and `await expectAccessible(page)` to only the login
visual test, after `expectVisual`. Run:

```bash
devtool run -- cargo xtask e2e-local auth.spec.ts
```

If the real login state has findings, fix each at its owning product source and
rerun until the unmodified state passes.

Then temporarily mutate the mounted login DOM after `expectVisual` and before
the helper call: set a visual-neutral invalid ARIA attribute value such as
`aria-checked="bogus"` on the username input. Rerun the same command. Expected:
the screenshot passes first, then axe fails; the report contains the rule ID,
impact, help URL, and affected selector. Remove the temporary mutation and
rerun. Expected: PASS. Confirm the temporary defect is absent from the diff.

- [x] **Step 4: Extend the contract to the other three core states**

Add the same second tag and post-screenshot helper call to:

- the public timeline/default-theme test in `theme.spec.ts`;
- the authenticated `/app` cockpit test in `authed-flash.spec.ts`;
- the empty `/posts/new` composer test in `posts.spec.ts`.

For the composer, call axe immediately after `expectVisual` and before filling
whitespace or real body text. Run the complete Chromium local loop:

```bash
devtool run -- cargo xtask e2e-local
```

For every reported violation, trace the target to its actual Leptos markup or
style owner, make the smallest semantic source fix, and rerun the owning spec
until all four scans pass. Do not alter the helper policy or call-site scope to
make a finding disappear.

If a fix changes visible output, regenerate only through:

```bash
devtool run -- cargo xtask e2e-local --update-visual-snapshots
```

Review every changed Chromium and Firefox PNG, then rerun normal
`cargo xtask e2e-local` so exact comparison passes without update mode. If
rendering did not change, no PNG may change.

- [x] **Step 5: Document the accessibility workflow**

Add `#### Accessibility workflow` beside the existing visual workflow in
`CONTRIBUTING.md`. State:

- the current four tagged states and WCAG 2.2 A/AA axe target;
- the zero-violation/no-suppression policy;
- the extension recipe: choose a stable existing behavioral test, retain its
  readiness checks, add `@accessibility`, and call `expectAccessible(page)` only
  after the complete intended state is mounted;
- axe covers only machine-checkable findings and does not replace keyboard,
  screen-reader, manual, or inclusive-user assessment.

Run:

```bash
devtool run -- prettier -w CONTRIBUTING.md end2end/package.json end2end/tests/accessibility.ts end2end/tests/auth.spec.ts end2end/tests/theme.spec.ts end2end/tests/authed-flash.spec.ts end2end/tests/posts.spec.ts docs/superpowers/specs/2026-08-14-issue-309-axe-accessibility-spec.md docs/superpowers/plans/2026-08-14-issue-309-axe-accessibility-plan.md
```

- [x] **Step 6: Gate, stage, and commit the completed issue**

Tick Steps 1–6, then run `devtool run -- cargo xtask check` with the refreshed
`E2E_TYPES_NODE_MODULES` environment from Step 1.

Inspect and stage any mechanical fixes from the gate. Stage the dependency,
helper, four call sites, product remediations (if any), reviewed PNG updates (if
any), `CONTRIBUTING.md`, the approved specification, and this plan. Verify the
staged tree contains no temporary negative probe, fake Nix hash, exclusion,
disabled rule, or unrelated change. Invoke `git commit` with the same refreshed
environment so the pre-commit hook checks the updated dependency closure.
Commit:

```text
test(e2e): gate core states with axe (#309)
```

## Shipping verification

After Task 1 is committed and reviewed, `jaunder-ship` rebases and archives the
planning documents, commits those non-behavioral changes, confirms the tree is
clean, and runs:

```bash
devtool run -- cargo xtask validate
```

Use the refreshed `E2E_TYPES_NODE_MODULES` environment from Step 1 if the agent
parent environment was not re-entered. Expected: all static checks and all four
`{sqlite,postgres}×{chromium,firefox}` combinations pass; the four selected
states report zero violations everywhere and existing visual comparisons remain
exact.

If validation fails, return to `jaunder-iterate`, reopen the owning
implementation step, fix the source, rerun its focused check, tick it again, run
the per-commit gate, commit the correction, and restart shipping from a clean
tree. Never use `--allow-dirty`.
