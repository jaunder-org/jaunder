# Spec — rename `hydrationHeavy*` e2e timeout helpers (#224)

> **Rebase note (2026-07-05, post-#153/PR#256).** This spec was written against
> pre-#256 `main` and describes 5 symbols spanning `playwright.config.ts`. #256
> then consolidated the config and removed its per-project timeout scaling, so
> the two config-local symbols no longer exist — the delivered scope is **4
> symbols** in `fixtures.ts` + call sites. `end2end/CLAUDE.md` (untracked) is
> out; `CONTRIBUTING.md` was added after a repo-wide sweep. See the plan's
> rebase note for the authoritative delta.

## Problem

`end2end/tests/fixtures.ts` and `end2end/playwright.config.ts` expose a family
of timeout helpers named `hydrationHeavy*`. Post the leptos-CSR cutover (#180)
there is **no hydration** — the phase these budgets cover is the CSR WASM mount,
and the Firefox/WebKit slowness is inherent WASM/JS execution, not hydration
(see `docs/observability.md` → "#155 — post-CSR Firefox e2e tax"). The names are
actively misleading. The budgets themselves are correct and worker-contention
aware (`Math.max(browserScale, workerContentionScale)`); this is a
**naming-only** change, no behavior change.

Deferred from #155. The companion task (fix the `run-e2e-trace-analysis`
out-path parser) is **obsolete** — that tooling was retired by #33; scope is the
rename only (confirmed by the 2026-07-04 triage rescope and issue comment).

## Scope — the 5 symbols

The issue names three; investigation found **five** distinct `hydrationHeavy*`
identifiers. All are renamed to a consistent `slowBrowser*` scheme (chosen at
the design gate; `slowBrowser` is the headline factor — Firefox/WebKit WASM
slowness — with worker-contention as the secondary `max` guard):

| Old                                                    | New                                                 | Kind        | Defined in                                                                |
| ------------------------------------------------------ | --------------------------------------------------- | ----------- | ------------------------------------------------------------------------- |
| `hydrationHeavyTimeoutScale` (2.2)                     | `slowBrowserTimeoutScale`                           | const       | `fixtures.ts` **and** `playwright.config.ts` (each declares its own copy) |
| `hydrationHeavyFirstNavigationScale` (2.6)             | `slowBrowserFirstNavigationScale`                   | const       | `fixtures.ts`                                                             |
| `hydrationHeavyTimeoutMs(testInfo, ms)`                | `slowBrowserTimeoutMs(testInfo, ms)`                | exported fn | `fixtures.ts`                                                             |
| `hydrationHeavyFirstNavigationTimeoutMs(testInfo, ms)` | `slowBrowserFirstNavigationTimeoutMs(testInfo, ms)` | exported fn | `fixtures.ts`                                                             |
| `hydrationHeavyProjectTimeoutMs`                       | `slowBrowserProjectTimeoutMs`                       | const       | `playwright.config.ts`                                                    |

## Files touched

**Code (call sites + definitions):**

- `end2end/tests/fixtures.ts` — definitions + the two consts + the stale "(Name
  kept as `hydrationHeavy*` for now …)" comment block, which is deleted (the
  rename it promised is now done).
- `end2end/playwright.config.ts` — local `slowBrowserTimeoutScale` +
  `slowBrowserProjectTimeoutMs`.
- `end2end/tests/helpers.ts` — doc-comment references.
- Every `end2end/tests/*.spec.ts` call site: `admin-site`, `atompub`, `auth`,
  `authed-flash`, `email`, `feeds`, `media`, `password_reset`, `posts`,
  `unicode-slug`, `visibility`.

**Live docs (kept accurate):**

- `docs/observability.md` — usage section + the `slowBrowserTimeoutScale = 2.2`
  reference.
- `docs/adr/0012-environment-aware-timeouts.md` — the helper-API reference lines
  (the decision/rationale prose is unchanged; only the two symbol names update
  so the ADR keeps naming the live API correctly).
- `end2end/CLAUDE.md` — the "Use `hydrationHeavy*`" convention rules.

**Explicitly NOT touched (historical records):**

- `docs/archive/**` and `docs/superpowers/plans|specs/**` from prior cycles —
  these record past state; rewriting them would falsify history. They keep the
  old names.

## Approach

A mechanical, whole-word rename. Because the tokens are unique and unambiguous
(`hydrationHeavy` prefix appears nowhere else as a substring of an unrelated
identifier), a global whole-word substitution across the in-scope files is safe.
The bare word `hydrationHeavy` appearing in prose/comments (fixtures.ts line
~89, observability.md) is reworded, not blind-substituted, since it reads as
English there.

## Acceptance criteria

1. `rg 'hydrationHeavy' end2end docs/observability.md docs/adr end2end/CLAUDE.md`
   returns **zero** matches (archives excluded).
2. No `slowBrowser*` symbol is left undefined or unused — every call site
   resolves to the renamed definition; `tsc` / the e2e lint passes.
3. Behavior is identical: the scale values (2.2, 2.6), the `max(...)` logic, and
   all `chromiumBudgetMs` arguments are byte-for-byte unchanged. This is a
   rename only.
4. `cargo xtask validate` (or at minimum the e2e static/lint checks + a
   representative e2e run) is green.

## Out of scope

- Any change to the budget _values_ or the contention model.
- Touching historical archive/superpowers docs.
- The obsolete trace-analysis parser fix (retired by #33).
