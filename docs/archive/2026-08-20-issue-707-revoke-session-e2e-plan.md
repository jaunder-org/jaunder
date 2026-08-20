# Issue #707 Revoke Session E2E Plan

## For agentic workers

Execute this plan task-by-task with `jaunder-iterate`. Do not skip the
`sqlite × chromium` e2e capture before regenerating server-fn coverage: the
compared coverage snapshot must be derived from real trace evidence, not edited
by hand.

## Goal

Add browser-flow coverage for `web::sessions::revoke`, remove its server-fn
coverage allowlist entry, and prove the static gate remains green.

## Approved spec

`docs/superpowers/specs/2026-08-20-issue-707-revoke-session-e2e-spec.md`

## Task 1: Add the browser revoke-session flow

1. Create `end2end/tests/sessions.spec.ts` for Sessions UI behavior.
2. Import:
   - `test`, `expect` from `./fixtures`;
   - `BASE_URL`, `click`, `goto`, `signInAs`, `signInAsNewUser`,
     `waitForSelector` from `./helpers`;
   - `navigateInApp` from `./navigate`;
   - `SEL` from `./selectors`.
3. Add one test, titled to name revoke-session coverage, that:
   - signs the default page in as a fresh user with `signInAsNewUser(page)`;
   - creates `otherContext = await tracedContext()`,
     `otherPage = await otherContext.newPage()`, and signs that page in as the
     same user with `signInAs(otherPage, username)`;
   - boots `otherPage` to `/` with `goto(otherPage, "/")` and waits for
     authenticated UI via `SEL.logoutLink`;
   - boots the default page to `/sessions` with `goto(page, "/sessions")`;
   - locates the current row and the non-current row under the sessions list,
     using `(current)` to distinguish same-label seeded sessions;
   - clicks the non-current row's `Revoke` button with the shared `click`
     helper;
   - asserts the non-current row disappears and the current row remains;
   - drives `otherPage` through an in-app navigation to `/app` with
     `navigateInApp`, expecting `/login` and `SEL.username`, proving the revoked
     session reconciles as logged out;
   - closes `otherContext` in `finally`.
4. Do not use raw `browser.newContext()` or a direct `/api/sessions/revoke`
   POST.
5. Do not add a custom timeout unless the test proves the ambient budget is
   insufficient.

Expected: the new spec directly drives `/api/sessions/revoke` through the
browser and is trace-attributable.

## Task 2: Run the targeted e2e and regenerate coverage

1. Run the authoritative capture-producing combo:

   ```bash
   devtool run -- cargo xtask e2e sqlite chromium
   ```

   If the run fails only because `server-fn-coverage-verify` sees expected
   snapshot drift, confirm the new test passed and the capture exists; then
   continue to regeneration. Any Playwright, panic, build, or non-coverage
   failure must be fixed before regenerating.

2. Run:

   ```bash
   devtool run -- cargo xtask server-fn-coverage regenerate
   ```

3. Verify generated changes:
   - `docs/coverage/server-fns.json` adds `sessions::revoke` under `covered`;
   - `docs/coverage/server-fns-evidence.json` has a `sessions::revoke` entry
     with the new test title;
   - no unrelated coverage regression is hidden by the generated diff.

Expected: coverage artifacts reflect real trace evidence for `sessions::revoke`.

## Task 3: Remove stale allowlist entry and gate

1. Remove the `sessions::revoke` object from
   `docs/coverage/server-fns-allowlist.json`; if it was the only entry, the file
   becomes `[]`.
2. Run:

   ```bash
   devtool run -- cargo xtask check
   ```

3. Inspect the staged/unstaged diff for only intended changes:
   - new `end2end/tests/sessions.spec.ts`;
   - regenerated `docs/coverage/server-fns.json`;
   - regenerated `docs/coverage/server-fns-evidence.json`;
   - edited `docs/coverage/server-fns-allowlist.json`;
   - the approved spec and this plan.
4. Commit with a common-commit message referencing #707.

Expected: `cargo xtask check` is green with `sessions::revoke` covered and no
allowlist bypass.

## Task 4: Archive and ship

1. Move the approved spec and plan to `docs/archive/`.
2. Run `devtool run -- cargo xtask check`.
3. Commit the archive move.
4. Rebase on `origin/main`.
5. Run `devtool run -- cargo xtask validate` because the change adds e2e
   behavior and coverage artifacts.
6. Push, open a PR with a common-commit title, and monitor with
   `cargo xtask pr watch`.
7. Stop for merge approval.
8. After approved merge succeeds, verify #707 is closed by the PR and the
   Jaunder Backlog project Status is `Done`.

Expected: #707 lands with browser-flow evidence and the allowlist removed.
