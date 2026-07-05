# Plan — rename `hydrationHeavy*` e2e timeout helpers (#224)

Spec:
[`2026-07-05-issue-224-rename-hydration-heavy-timeout-helpers.md`](../specs/2026-07-05-issue-224-rename-hydration-heavy-timeout-helpers.md)

> **Rebase note (2026-07-05, post-#153/PR#256).** #256 consolidated the
> Playwright config: it dropped the per-project timeout scaling, so the
> config-local `hydrationHeavyTimeoutScale` and `hydrationHeavyProjectTimeoutMs`
> **no longer exist** — the original **Task 2 (`playwright.config.ts`) is
> removed** and the rename map shrinks from 5 symbols to **4**. Separately,
> `end2end/CLAUDE.md` is an **untracked** local file (no `origin/main` history;
> not in this worktree), so it is **out of scope** for this branch's commit.
> Everything else is unchanged.

## Review header

**Goal:** rename the 4 remaining `hydrationHeavy*` e2e timeout symbols →
`slowBrowser*`. Naming-only; zero behavior change.

**Scope**

- _In:_ the 4 symbols (see rename map) across `end2end/` code + call sites, and
  live-doc references (`observability.md`, `adr/0012`).
- _Out:_ `playwright.config.ts` (no `hydrationHeavy*` left post-#256);
  `end2end/CLAUDE.md` (untracked, not on branch); budget values / contention
  model; `docs/archive/**` and prior-cycle `docs/superpowers/**` (historical);
  the obsolete #33 parser fix.

**Tasks**

1. Rename definitions in `fixtures.ts` (consts + 2 fns; delete the stale "name
   kept for now" comment).
2. Rename every call site (`helpers.ts` doc-comments + 11 `*.spec.ts`).
3. Update live docs (`observability.md`, `adr/0012`).
4. Verify (grep-clean + e2e lint/tsc + representative e2e run) and commit —
   **one atomic commit** for tasks 1–3.

**Key risks / decisions**

- **Atomicity:** a rename split across commits leaves broken imports; tasks 1–3
  are staged together and land as a single commit after verification.
- **Token precision:** substitute the whole token `hydrationHeavy…` only. The
  unrelated hydration mechanics in `fixtures.ts` (`data-hydrated`,
  `waitForHydration`, `hydratedMs`, `__jaunderRecordHydration`,
  `notifyHydration`, `__jaunderHydrationNotified`) contain `hydrat` but never
  `hydrationHeavy` — they must stay untouched.
- **Prose vs identifier:** the bare word `hydrationHeavy` in comments/prose
  (`fixtures.ts` comment block, `observability.md`) is reworded to read as
  English, not blind-substituted.

**For agentic workers:** execute with `jaunder-iterate`; this change is small
and mechanical enough to do inline (no `jaunder-dispatch` subagent needed).

## Global constraints

- Rename map (exhaustive — nothing else changes):
  - `hydrationHeavyTimeoutScale` → `slowBrowserTimeoutScale`
  - `hydrationHeavyFirstNavigationScale` → `slowBrowserFirstNavigationScale`
  - `hydrationHeavyTimeoutMs` → `slowBrowserTimeoutMs`
  - `hydrationHeavyFirstNavigationTimeoutMs` →
    `slowBrowserFirstNavigationTimeoutMs`
- No `Co-Authored-By` trailer on the commit.

---

## Task 1 — `end2end/tests/fixtures.ts` definitions

- [x] Rename consts `hydrationHeavyTimeoutScale` (2.2) and
      `hydrationHeavyFirstNavigationScale` (2.6) to their `slowBrowser*` names;
      values unchanged.
- [x] Rename the two exported fns `hydrationHeavyTimeoutMs` /
      `hydrationHeavyFirstNavigationTimeoutMs` and their internal
      `hydrationHeavyTimeoutScale` / `hydrationHeavyFirstNavigationScale`
      references.
- [x] Rename the internal call sites (`user` fixture → `register(...)`,
      `verifiedUser` fixture → `setTimeout` + `login(...)`).
- [x] Delete the now-obsolete comment tail "(Name kept as `hydrationHeavy*` for
      now; there is no hydration on CSR — the rename is a follow-up in this
      cycle.)" — the rename it defers is now done. Keep the rest of the
      two-reasons budget comment; reword its lead-in if it references the old
      name.
- [x] Update the file's top doc-comment ("hydration-aware timeout scalers") to
      describe them accurately (slow-browser / contention budgets).

## Task 2 — call sites

- [x] `end2end/tests/helpers.ts` — update the three doc-comment references.
- [x] Rename in every spec: `admin-site`, `atompub`, `auth`, `authed-flash`,
      `email`, `feeds`, `media`, `password_reset`, `posts`, `unicode-slug`,
      `visibility` (`.spec.ts`). Whole-token substitution.

## Task 3 — live docs

- [x] `docs/observability.md` — usage section + the
      `slowBrowserTimeoutScale =     2.2` reference + the bare `hydrationHeavy*`
      prose, reworded to read naturally.
- [x] `docs/adr/0012-environment-aware-timeouts.md` — the two helper-API
      reference lines; rationale prose unchanged.
- [x] `CONTRIBUTING.md` — the helper-usage bullet (~line 248). Found by the
      repo-wide sweep; not in the original scan, but a live tracked guide that
      must stay accurate.

## Task 4 — verify + commit

- [x] `rg 'hydrationHeavy' end2end docs/observability.md docs/adr` → **zero**
      matches.
- [x] Confirm no orphan: `rg 'slowBrowser' end2end` shows every call site
      resolves; no lingering old token anywhere in scope.
- [x] Run the e2e static/lint + typecheck (the `check` path that covers
      `end2end/` — TS lint/tsc), plus a representative e2e combo to prove the
      renamed helpers still drive real tests. Full `cargo xtask validate` before
      ship.
- [x] Stage tasks 1–3 together; commit as one atomic
      `refactor(e2e): rename hydrationHeavy* timeout helpers to slowBrowser* (#224)`
      (no `Co-Authored-By`). Pre-commit hook (`cargo xtask check`) must pass
      clean — see `jaunder-commit`.

## Self-review

- Rename map covers the 4 symbols that remain after the #256 rebase
  (`rg -o 'hydrationHeavy\w*'` over `end2end/` + live docs). ✔
- No behavior change: values, `Math.max` logic, `chromiumBudgetMs` args
  untouched. ✔
- Historical docs + untracked `end2end/CLAUDE.md` excluded; live docs included.
  ✔
