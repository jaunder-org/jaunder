# #873 — format-toggle e2e round-trip coverage: Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Spec:** `docs/superpowers/specs/2026-08-11-issue-873-format-toggle-e2e.md` —
read it for the _what_ and _why_; this plan is the _how_. Acceptance criteria
are cited as AC1–AC6.

**Goal:** Give the `.j-seg` format toggle round-trip e2e coverage on all three
surfaces it renders on — `/app`, `/posts/new`, and `/posts/:id/edit` — proving
the chosen format reaches the saved post, not just the CSS class.

**Architecture:** Test-only. One shared assertion helper in
`end2end/tests/posts.ts` navigates to a permalink and makes a two-sided
assertion on `.j-post-body` (`b` for Org, `em` for Markdown; the other absent).
Three tests each drive one surface and call it. A `format` option is added to
the existing `composePost` helper so a test can create a post in a chosen format
through the UI.

**Tech Stack:** Playwright + TypeScript (`end2end/`), driven by
`cargo xtask e2e-local` (fast loop) and `cargo xtask e2e <backend> <browser>`
(gate).

## Review header

**Scope — in:**

- `end2end/tests/selectors.ts` — one new `SEL` entry for the format buttons.
- `end2end/tests/posts.ts` — the `expectRenderedFormat` helper; a `format`
  option on `composePost`.
- `end2end/tests/posts.spec.ts` — two new tests, one existing test extended.

**Scope — out:** any change under `web/`, `common/`, `server/`, or `storage/`.
This plan touches `end2end/` only (spec Non-goals). No `PostFormat::Html`
coverage. No new `data-test` hooks in product markup. No separable follow-up
issues surfaced during the spec interview, so there is no issue-filing task.

**Tasks:**

1. `SEL.formatButton` + `expectRenderedFormat` + the `/posts/new` round-trip
   test (AC1, AC2).
2. `composePost`'s `format` option + the edit-page prefill-and-change test (AC3,
   AC4).
3. Extend the existing `/app` compact-composer test with its round-trip (AC5).
4. Run the full gate including the four-combo e2e matrix (AC6).

**Key risks / decisions:**

- **Task 2 is the fragile one.** It depends on the two-hop route to the edit URL
  (permalink → `.j-post-acts a:has-text("Edit")` → regex the id), and on the
  post being a **draft** — the editor renders a permalink link only when
  `published_at.is_none()` (spec D3, D4).
- **Task 2 is also the only Markdown-direction assertion**, which is what stops
  the helper's format argument from being decorative (spec D7, AC1).
- **Task 3's flash is time-bounded** (30 s `set_timeout`, cleared on input), so
  the href must be captured immediately after the wait (spec D6).
- **Per-task green is narrower than it looks.** `cargo xtask check` is static +
  clippy + coverage — it does **not** run e2e at all — and
  `cargo xtask e2e-local` runs a single combo (sqlite × chromium). So Tasks 1–3
  commit tests that only one of four combos has seen. **Task 4 is the real gate
  for this branch**, and a firefox- or postgres-only failure is expected to
  surface there rather than earlier.

## Global Constraints

- **Test-only change.** No file outside `end2end/` is modified. If a new test
  fails because product wiring is genuinely broken, stop and file an issue — do
  not fix product code in this branch (spec Non-goals).
- **The probe body is the literal `*emphasis*`** — the string whose rendering
  differs by format (`<em>emphasis</em>` Markdown, `<b>emphasis</b>` Org).
  Pinned by `common/src/render.rs:1647` (Markdown, `*emphasis*` → `<em>`) and
  `:1696` (`render_dispatches_org`, which renders a **bare** `*bold*` body — the
  same shape as the probe — and asserts `<b>`).
- **Assertions are two-sided:** expected element present with the probe text
  **and** the other format's element at count 0.
- **Never construct or reuse a permalink** — always read the `href` fresh off
  the summary/flash the save just produced (spec D6).
- **Formatting:** `prettier` from the devShell, invoked bare under
  `devtool run --`. Never `npx`.
- **Commits:** run `cargo xtask check` before each commit so the pre-commit hook
  passes clean (`jaunder-commit`). **No `Co-Authored-By` trailer.**

---

### Task 1: The shared helper and the `/posts/new` round-trip

**Files:**

- Modify: `end2end/tests/selectors.ts:10-34` (add one `SEL` entry)
- Modify: `end2end/tests/posts.ts` (append the helper; it imports `expect`,
  `goto`, `SEL` — all already imported at `:10-13`)
- Test: `end2end/tests/posts.spec.ts` (new test, placed next to the existing
  toggle test at `:715`)

**Interfaces:**

- Consumes: `goto`, `click`, `waitForSelector` from `./helpers`; `SEL` from
  `./selectors`; `expect`, `type Page` from `@playwright/test`.
- Produces — later tasks rely on both of these by these exact names:

  ```ts
  // end2end/tests/selectors.ts — inside the SEL object
  /** A `.j-seg` format-toggle button, by its visible label. The label is a
   *  literal union, not `string`: a casing typo (`"org"`) would otherwise
   *  compile and fail as a 30-second locator timeout. */
  formatButton: (label: "Markdown" | "Org") =>
    `.j-seg button:has-text("${label}")`,

  // end2end/tests/posts.ts
  export async function expectRenderedFormat(
    page: Page,
    permalinkHref: string,
    format: "markdown" | "org",
  ): Promise<void>;
  ```

- [x] **Step 1: Add the `SEL.formatButton` entry**

In `end2end/tests/selectors.ts`, inside the `SEL` object, add the doc comment
and entry shown in the Interfaces block above. `selectors.ts:5-7` says one-off
selectors stay inline; this one earns a constant because it is used in three
tests across ~10 call sites. It follows the existing arrow-function precedent,
`publishButton` at `selectors.ts:19`.

- [x] **Step 2: Write the `expectRenderedFormat` helper**

Append to `end2end/tests/posts.ts`:

```ts
/** Navigate to a saved post's permalink and assert it rendered in `format`.
 *
 *  The probe body `*emphasis*` renders differently per format — `<em>` in
 *  Markdown (`common/src/render.rs:1647`), `<b>` in Org (`:1696`, which renders
 *  a bare `*bold*` body, the same shape as the probe) — so the format the post
 *  was *saved* with is observable in the DOM. The assertion is two-sided on
 *  purpose: checking only that the expected element exists would pass on a page
 *  that rendered both, or neither (#873). */
export async function expectRenderedFormat(
  page: Page,
  permalinkHref: string,
  format: "markdown" | "org",
): Promise<void> {
  await goto(page, permalinkHref);
  const body = page.locator(".j-post-body");
  const [expectedTag, otherTag] = format === "org" ? ["b", "em"] : ["em", "b"];
  await expect(body.locator(expectedTag)).toHaveText("emphasis");
  await expect(body.locator(otherTag)).toHaveCount(0);
}
```

- [x] **Step 3: Write the failing `/posts/new` test**

Add to `end2end/tests/posts.spec.ts` with the other `/posts/new` tests — after
the draft test that ends at `:137`, before
`"published post renders at permalink"` at `:139`. (Not down at `:731`: that is
the `/app` block, and this test drives `/posts/new`.) Add `expectRenderedFormat`
to the existing `import { composePost, createPostViaApi } from "./posts";` at
`:17`.

```ts
test("full composer: format toggle round-trips to the rendered post", async ({
  registeredPage: page,
}) => {
  test.slow();
  await goto(page, "/posts/new");
  await waitForSelector(page, ".j-seg");

  const markdownBtn = page.locator(SEL.formatButton("Markdown"));
  const orgBtn = page.locator(SEL.formatButton("Org"));

  // Markdown is the default (ComposeState::default, compose_state.rs:54).
  await expect(markdownBtn).toHaveClass(/is-selected/);
  await expect(orgBtn).not.toHaveClass(/is-selected/);

  await page.fill(SEL.postBody, "*emphasis*");
  await click(page, SEL.formatButton("Org"));
  await expect(orgBtn).toHaveClass(/is-selected/);
  await expect(markdownBtn).not.toHaveClass(/is-selected/);

  await click(page, SEL.publishButton("true"));
  await waitForSelector(page, SEL.saveSummary);

  const permalinkHref = (await page
    .locator(SEL.saveSummary)
    .locator('[data-test="permalink-link"]')
    .getAttribute("href"))!;
  expect(permalinkHref).toBeTruthy();

  // The class moved *and* the saved post really is Org.
  await expectRenderedFormat(page, permalinkHref, "org");
});
```

- [x] **Step 4: Run it, verify it passes**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-873-format-toggle-e2e -- cargo xtask e2e-local posts.spec.ts`
Result: **PASS** — 37 passed, the new test running as `posts.spec.ts:139`.
Expected: PASS. Unlike a unit-test task there is no red phase to stage — this is
coverage of behavior believed to already work, so a **failure here means a real
product bug**: stop, do not "fix" it in `end2end/`, and report it (Global
Constraints).

- [x] **Step 5: Format, gate, and commit**

```bash
devtool run -- prettier -w end2end/tests/selectors.ts end2end/tests/posts.ts end2end/tests/posts.spec.ts
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-873-format-toggle-e2e -- cargo xtask check
git add end2end/tests/selectors.ts end2end/tests/posts.ts end2end/tests/posts.spec.ts
git commit -m "test(e2e): round-trip the format toggle on /posts/new (#873)"
```

---

### Task 2: The edit page — prefill and change

**Files:**

- Modify: `end2end/tests/posts.ts:56-73` (add the `format` option to
  `composePost`)
- Test: `end2end/tests/posts.spec.ts` (new test, after Task 1's)

**Interfaces:**

- Consumes: `expectRenderedFormat` and `SEL.formatButton` from Task 1.
- Produces:

  ```ts
  // composePost's options object gains one optional field:
  //   format?: "markdown" | "org"   // clicks the .j-seg button before submitting;
  //                                 // omitted leaves the Markdown default
  ```

- [x] **Step 1: Add the `format` option to `composePost`**

In `end2end/tests/posts.ts`, widen the `opts` type at `:58` to
`{ body: string; summary?: string; slug?: string; publish: boolean; format?: "markdown" | "org" }`
and, inside the `withTimedAction` callback, insert the format click **after**
the body fill (`:62`) and **before** the publish click (`:69`):

```ts
if (opts.format !== undefined) {
  await click(
    page,
    SEL.formatButton(opts.format === "org" ? "Org" : "Markdown"),
  );
}
```

Extend the JSDoc at `:51-55` to mention that `format` picks the `.j-seg` toggle
and that omitting it leaves the composer's Markdown default. Existing call sites
pass no `format` and are unaffected.

No explicit `waitForSelector(".j-seg")` is needed here, unlike in the specs: the
preceding `page.fill(SEL.postBody, …)` already blocks on the same `Suspense`
gate that produces the toggle (`component.rs:743-754`), and `click` auto-waits
for actionability regardless.

- [x] **Step 2: Write the failing edit-page test**

Add to `end2end/tests/posts.spec.ts` next to the other edit-page tests, after
`"authenticated user can edit a draft post"` (which ends around `:205`) — it
reuses that test's two-hop idiom, so they read well together.

```ts
test("edit page: format toggle prefills from the post and round-trips a change", async ({
  registeredPage: page,
}) => {
  test.slow();
  // A *draft*, in *Org*. Draft because the editor renders a permalink link only
  // for an unpublished post (component.rs:1307); Org because the composer's
  // default is Markdown, so only a non-default format proves the prefill fired
  // (compose_state.rs:54 vs :104).
  const summary = await composePost(page, {
    body: "*emphasis*",
    publish: false,
    format: "org",
  });

  const draftHref = (await summary
    .locator('[data-test="permalink-link"]')
    .getAttribute("href"))!;
  expect(draftHref).toBeTruthy();

  // Two-hop route to the edit URL: no id is exposed by the summary or the
  // permalink, so read it off the PostCard's Edit affordance (posts.spec.ts:178-189).
  await goto(page, draftHref);
  const editLink = page.locator('.j-post-acts a:has-text("Edit")');
  await editLink.waitFor();
  const postId = (await editLink.getAttribute("href"))!.match(
    /\/posts\/(\d+)\/edit/,
  )![1];

  await goto(page, `/posts/${postId}/edit`);
  await waitForSelector(page, ".j-seg");

  // Guard: confirm the *body* seeded too. `dispatch_update` silently no-ops on a
  // blank body (component.rs:1096-1100), so a broken body prefill would surface
  // far below as a save that never produces a summary — a full-timeout death with
  // no clue why. Failing here instead names the cause.
  await expect(page.locator(SEL.postBody)).toHaveValue("*emphasis*");

  const markdownBtn = page.locator(SEL.formatButton("Markdown"));
  const orgBtn = page.locator(SEL.formatButton("Org"));

  // Prefill: the toggle shows the *stored* format, which the default cannot produce.
  await expect(orgBtn).toHaveClass(/is-selected/);
  await expect(markdownBtn).not.toHaveClass(/is-selected/);

  // Switch back to Markdown and save.
  await click(page, SEL.formatButton("Markdown"));
  await expect(markdownBtn).toHaveClass(/is-selected/);
  await expect(orgBtn).not.toHaveClass(/is-selected/);

  await click(page, SEL.publishButton("false"));
  await waitForSelector(page, SEL.saveSummary);

  // Re-read the href from the save that just happened; never reuse draftHref.
  const savedHref = (await page
    .locator(SEL.saveSummary)
    .locator('[data-test="permalink-link"]')
    .getAttribute("href"))!;
  expect(savedHref).toBeTruthy();

  await expectRenderedFormat(page, savedHref, "markdown");
});
```

- [x] **Step 3: Run it, verify it passes**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-873-format-toggle-e2e -- cargo xtask e2e-local posts.spec.ts`
Expected: PASS, both this test and Task 1's. As in Task 1, a failure is a
product-bug signal, not a cue to loosen the assertion. If the failure is
specifically the two-hop lookup (`editLink` never appears), check whether the
pre-existing test at `:167` — which uses the same route — also fails; if it
does, the breakage is shared infrastructure, not this test.

Result: **PASS on re-run — 38 passed.** The first run failed on the body-seed
guard, which is exactly the localization it was added for: the guard asserted
the exact string `*emphasis*` but the seeded textarea holds `*emphasis*\n`,
because the body is canonicalized on save (`normalize_body_whitespace` appends a
trailing newline). That is correct product behavior, not a bug, so the guard was
loosened to `/^\*emphasis\*\s*$/` — it exists to prove the body arrived at all,
and pinning exact whitespace only made it brittle. No assertion about the format
itself was weakened.

- [x] **Step 4: Format, gate, and commit**

```bash
devtool run -- prettier -w end2end/tests/posts.ts end2end/tests/posts.spec.ts
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-873-format-toggle-e2e -- cargo xtask check
git add end2end/tests/posts.ts end2end/tests/posts.spec.ts
git commit -m "test(e2e): cover format prefill and change on the edit page (#873)"
```

---

### Task 3: The compact composer round-trip

**Files:**

- Modify: `end2end/tests/posts.spec.ts:715-731` (extend the existing test)

**Interfaces:**

- Consumes: `expectRenderedFormat` and `SEL.formatButton` from Task 1. Produces
  nothing.

- [x] **Step 1: Rewrite the existing `:715` test to round-trip**

Replace `test("inline composer: format toggle switches active button", …)`
entirely, **including its name** — it now publishes and follows a permalink, so
the old name undersells it. AC5 requires keeping the assertions, not the title.
The default-state and class-flip assertions are kept in intent but re-expressed
through `SEL.formatButton`; the publish-and-verify half is new.

```ts
test("inline composer: format toggle round-trips to the rendered post", async ({
  registeredPage: page,
}) => {
  test.slow();
  await goto(page, "/app");
  await waitForSelector(page, ".j-composer");

  // Markdown is active by default.
  const markdownBtn = page.locator(SEL.formatButton("Markdown"));
  const orgBtn = page.locator(SEL.formatButton("Org"));
  await expect(markdownBtn).toHaveClass(/is-selected/);
  await expect(orgBtn).not.toHaveClass(/is-selected/);

  // Click Org to switch.
  await page.fill('.j-composer textarea[name="body"]', "*emphasis*");
  await click(page, SEL.formatButton("Org"));
  await expect(orgBtn).toHaveClass(/is-selected/);
  await expect(markdownBtn).not.toHaveClass(/is-selected/);

  // ...and the choice reaches the saved post, not just the highlight.
  await click(page, '.j-composer button[name="publish"][value="true"]');

  // The flash *is* the permalink anchor (component.rs:717-718) and it is
  // transient — a 30s set_timeout (:696) plus an on_input reset (:710) — so
  // capture the href immediately, before any further interaction.
  const flashLink = page.locator(".j-composer p.success a");
  await flashLink.waitFor();
  const permalinkHref = (await flashLink.getAttribute("href"))!;
  expect(permalinkHref).toBeTruthy();

  await expectRenderedFormat(page, permalinkHref, "org");
});
```

- [x] **Step 2: Run it, verify it passes**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-873-format-toggle-e2e -- cargo xtask e2e-local posts.spec.ts`
Result: **PASS — 38 passed**, all three round-trip tests among them. The flash
href was captured without incident on sqlite × chromium; the four-combo check is
Task 4. Expected: PASS — all three tests. A null `permalinkHref` here means the
flash expired or was cleared before the read; that is the known flakiness vector
(spec D6), so re-check the capture happens directly after `waitFor()`.

- [x] **Step 3: Format, gate, and commit**

```bash
devtool run -- prettier -w end2end/tests/posts.spec.ts
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-873-format-toggle-e2e -- cargo xtask check
git add end2end/tests/posts.spec.ts
git commit -m "test(e2e): round-trip the compact composer's format toggle (#873)"
```

---

### Task 4: Full-matrix verification (AC6)

**Files:** none — verification only.

**Interfaces:** none.

- [x] **Step 1: Run the full local gate**

Run, in Bash background mode (this is long and cold):
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-873-format-toggle-e2e -- cargo xtask validate`
Expected: PASS, including all four `{sqlite,postgres}×{chromium,firefox}` e2e
checks.

Result: **PASS on all four combos — re-run in full after the post-review
refactor**, since the first green predated it and the refactor touched the
delivered tests. Run as `validate --no-e2e` (green) plus the
four `cargo xtask e2e <backend> <browser>` checks individually — the same
derivations `validate` drives, split only because each run exceeded this
session's foreground command timeout. Neither firefox combo showed the timing
sensitivity the plan flagged as most likely: the compact composer's flash href
and the edit page's `Suspend`-gated prefill both held.

- [x] **Step 2: On a red combo, read the scoped failure log**

Per `CONTRIBUTING.md:351`, read the scoped diagnostics under
`.xtask/diagnostics/e2e-<backend>-<browser>/` rather than re-running blind. A
failure on one browser only is the signature of a timing assumption — the likely
suspects are the compact composer's flash expiry (Task 3) and the edit page's
`Suspend`-gated prefill (Task 2), both of which are already `await`-guarded but
are where to look first.

- [x] **Step 3: No commit**

This task produces no diff. If Step 2 required a fix, that fix is committed
against the task that owns the test, with the same gate-then-commit sequence. No
fix was needed.

---

## Base moved before landing — the tests were rebuilt on #867's conventions

While this branch was in flight, #867 landed ADR-0111: **an e2e page boots once**.
`registeredPage` now takes its entry path, `composePost` no longer navigates, in-app
moves go through `navigateInApp`, and a runtime budget fails any page that loads a
second document without declaring why. Every task snippet below predates that and uses
repeated `goto`, which the budget now rejects.

The work was rebuilt on the new base rather than replayed. What changed in shape — the
acceptance criteria are untouched:

- Each test **enters** at its surface (`await registeredPage("/posts/new")` /
  `("/app")`) instead of `goto`-ing there.
- `expectRenderedFormat` no longer navigates; it asserts on the post already displayed.
  Navigation is the caller's, in-app.
- `permalinkFromSummary` + `gotoEditPage` became `followPermalink` + `openEditor`, both
  built on `navigateInApp` with a `ready` barrier rather than a document load.
- The `/app` test moves on its flash anchor with `navigateInApp` directly: the compact
  composer has no `.j-save-summary`, so it is the one surface `followPermalink` cannot
  serve.

## Post-review refactor — the task snippets above are superseded

A code review after Task 4 found duplication in the delivered tests, and the fixes
(commit `858455f6`) moved the code away from the snippets embedded in Tasks 1–3. The
tasks' **intent** is unchanged and every acceptance criterion still holds; only the
shapes differ. Read the code, not the snippets, where they disagree:

- `expectRenderedFormat` is table-driven off a `FORMAT` map (`markdown → em`,
  `org → b`) instead of a `[expectedTag, otherTag]` destructure. Still two-sided.
- The probe body is exported as `FORMAT_PROBE_BODY` rather than literalled at each
  call site — the helper asserts on that body's rendering and cannot verify the caller
  composed it, so the two are now named together.
- The save-summary permalink read is `permalinkFromSummary`, and the edit test's
  two-hop route is `gotoEditPage` — both extracted into `posts.ts`.
- Task 3's test uses `SEL.formatButton` and `.j-composer`-scoped `SEL` entries, not raw
  literals.
- Task 2's body guard shipped as `/^\*emphasis\*\s*$/`, not `toHaveValue("*emphasis*")`
  — see that task's Step 3 result for why.

`gotoEditPage` currently has one call site; five other inlined copies of the same
two-hop remain in `posts.spec.ts`. Migrating them is filed separately rather than
folded in here.

## Self-review

**Spec coverage:** AC1 → Task 1 Step 2 (helper, two-sided) plus Task 2's
`"markdown"` call making the argument load-bearing. AC2 → Task 1 Step 3. AC3 →
Task 2 Step 2 (Org draft, prefill assertion). AC4 → Task 2 Step 2 (switch, save,
re-read, Markdown assertion). AC5 → Task 3 Step 1. AC6 → Task 4. Spec D1–D8 are
each realized: D1 (class + render assertions in all three tests), D2/D3 (Org
draft), D4 (two-hop route), D5 (`:715` extended not deferred), D6 (fresh href
reads; immediate flash capture), D7 (helper in `posts.ts`, three separate
tests), D8 (no ADR task — none needed).

**Placeholder scan:** no TBD/TODO; every test is written out in full; every run
step names an exact command and an expected result.

**Type consistency:** `expectRenderedFormat(page, permalinkHref, format)` and
`SEL.formatButton(label)` are declared once in Task 1's Interfaces block and
called with matching arity and argument types in Tasks 1, 2, and 3.
`composePost`'s new `format` field uses the same `"markdown" | "org"` union as
the helper.
