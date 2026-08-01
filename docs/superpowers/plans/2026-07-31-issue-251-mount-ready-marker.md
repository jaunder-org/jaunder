# Mount-Readiness Marker Rename — Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with jaunder-iterate
> (delegating individual tasks to a subagent via jaunder-dispatch when useful).
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rename the CSR mount-readiness signal off the "hydration" misnomer,
end to end.

**Architecture:** A four-commit rename. Commit 1 moves every _symbol_ (the DOM
attribute, the helper, the module, the OTel span, the fixture plumbing) in one
atomic cross-language change, because the Rust `inline_js` attribute and the TS
selector must agree or every e2e test times out. Commits 2–3 clean the prose and
the docs. Commit 4 regenerates the coverage evidence after the renamed test
title exists.

**Tech Stack:** Rust (`csr` crate, `wasm_bindgen` `inline_js`), TypeScript
(Playwright e2e), `cargo xtask` gate.

**Spec:**
[`docs/superpowers/specs/2026-07-31-issue-251-mount-ready-marker.md`](../specs/2026-07-31-issue-251-mount-ready-marker.md)
— decisions D1–D9 and acceptance criteria AC1–AC7 are referenced by ID below and
not restated.

## Global Constraints

- **Stem is `mount`/`mounted`** (D1) — `data-mounted`, `waitForMount`,
  `mount.ts`, `wait.mount`, `mountedMs`, `__jaunderRecordMount`. Never introduce
  a third vocabulary.
- **Zero residue outranks explanatory prose** (D9). No comment may retain the
  word "hydration" to explain the old name; rewrite the sentence instead.
- **Out of scope, do not touch** (spec "Out of scope"): `docs/adr/**`,
  `docs/archive/**`, `docs/web-style-guide.md`,
  `docs/issue-177-csr-spike-findings.md`, `web/src/**`, `storage/src/posts.rs`,
  `server/tests/storage/mod.rs`, `xtask/src/server_fn_coverage/testdata/**`, the
  rest of `xtask/` beyond `steps/build_csr.rs`, `Cargo.lock`.
- **Per-commit gate:** run `devtool run -- cargo xtask check` before each commit
  so the pre-commit hook passes clean (**jaunder-commit**). While iterating,
  `devtool run -- cargo xtask check --no-test` is the fast loop — it runs
  `prettier` and `tsc`, which is what catches a missed TypeScript rename.
- **No `Co-Authored-By` trailer** on any commit.
- Commit subjects follow the repo's #224 precedent, e.g.
  `refactor(e2e): rename hydrationHeavy* timeout helpers to slowBrowser* (#224)`.

---

## Review header

**Scope — in:** the marker chain (`data-hydrated` attribute, `waitForHydration`,
`hydration.ts`, `wait.hydration` span), the `fixtures.ts` OTel plumbing, every
"hydration"-for-mount comment in `csr/` + `end2end/`,
`xtask/src/steps/build_csr.rs:5`, the two live/dated sites in
`docs/observability.md`, the `theme.spec.ts` test title, and the regenerated
coverage evidence.

**Scope — out:** everything in Global Constraints above. In particular, prose
about _real_ SSR hydration stays exactly as written.

**Tasks:**

1. Rename every symbol — attribute, module, helper, span, fixture plumbing — in
   one atomic cross-language commit.
2. Rewrite the concept-only prose in the five remaining files (incl. the
   `theme.spec.ts` test title).
3. Rename `wait.hydration` → `wait.mount` in `docs/observability.md`, in place,
   with a rename note.
4. Regenerate `docs/coverage/server-fns-evidence.json` from a fresh capture so
   the renamed test title lands.
5. Run the full gate and complete the untracked-doc ship checklist.

**Key risks / decisions:**

- **The `addInitScript` trap (Task 1, Step 5).** `fixtures.ts:463` passes a
  function to `page.addInitScript()`, which is **serialized and executed in the
  browser** — it cannot close over `MOUNTED_ATTR` from Node module scope. A
  naïve `import` + reference there throws `ReferenceError` in the page, in a
  diagnostics-only path that does **not** redden e2e. The constant must be
  passed as the `addInitScript` _argument_. Verified: the current init script
  references only browser globals, never module scope.
- **Task 1 is deliberately large, and two parts of it must not be split out.**
  `cargo xtask check` runs static + clippy + host/Nix tests but **never e2e**,
  so a commit that breaks either coupling below passes `check` and then times
  out every test under `validate`:
  1. the attribute pair — `csr/src/lib.rs`'s `setAttribute` literal ↔
     `MOUNTED_ATTR`/`MOUNTED_SELECTOR`;
  2. the recorder-binding name — `fixtures.ts`'s
     `exposeBinding("__jaunderRecordMount")` ↔ `mount.ts`'s `page.evaluate`
     lookup ↔ the init script's call.

  The rest of the task (the `hydratedMs → mountedMs` field rename, the Step 6
  comment edits) is _not_ coupled and could in principle be split; it is kept
  here only because splitting a mechanical rename across commits costs more
  review effort than it saves.

- **Task ordering is load-bearing.** The title rename (Task 2) must precede the
  capture (Task 4), or the regenerated evidence records the old title.
- **Task 4 is the expensive one:** a full `cargo xtask e2e sqlite chromium` run,
  and its regenerate churns ~66 unrelated evidence entries into the diff
  (accepted under D5).

---

### Task 1: Rename the marker chain (atomic, cross-language)

**Files:**

- Rename: `end2end/tests/hydration.ts` → `end2end/tests/mount.ts` (use `git mv`)
- Modify: `csr/src/lib.rs:10-19`
- Modify: `end2end/tests/fixtures.ts` — `:3`, imports at `:25-36`, `:70`,
  `:169`, `:446`, `:452-458`, `:463-505`, `:543`, `:722`, `:738-739`
- Modify: `end2end/tests/helpers.ts:16-18`, `:40`, `:44`, `:71`
- Modify: `end2end/tests/layout-shift.ts:4`, `:14`, `:32`, `:82`
- Modify: `end2end/tests/password_reset.spec.ts:6`, `:41`, `:46`
- Modify: `end2end/tests/feeds.spec.ts:2`, `:128`, `:200`
- Modify: `end2end/tests/auth.spec.ts:76-77`
- Modify: `end2end/tests/authed-cls.spec.ts:16`
- Modify: `end2end/tests/timeline-cls.spec.ts:9`, `:19`
- Test: none added. The contract here is the **type checker**
  (`devtool check tsc`) plus the AC greps; the behavioral proof is the e2e suite
  in Task 5. There is no behavior change to pin with a new test — a new
  assertion would only restate `waitForMount`'s existing use by 60+ specs.

**Interfaces:**

- Consumes: nothing from earlier tasks.
- Produces, from `end2end/tests/mount.ts`:
  - `export const MOUNTED_ATTR: string` — value `"data-mounted"`
  - `export const MOUNTED_SELECTOR: string` — value
    `` `body[${MOUNTED_ATTR}]` ``
  - `export async function waitForMount(page: Page, timeoutMs?: number): Promise<void>`
  - Re-exported from `helpers.ts` as `waitForMount` (replacing the
    `waitForHydration` re-export).

- [ ] **Step 1: Move the module and write it in full**

```bash
git mv end2end/tests/hydration.ts end2end/tests/mount.ts
```

Then replace the entire contents of `end2end/tests/mount.ts` with:

```ts
import type { Page } from "@playwright/test";
import { withTimedAction } from "./actions";

/**
 * The body attribute the CSR client sets once `mount_to_body` has run — the
 * suite's "app is mounted and interactive" signal. Counterpart of the literal in
 * `csr/src/lib.rs`'s `mark_ready` inline JS; the two must agree or every e2e test
 * times out. Declared once here so a rename touches one place (#251).
 */
export const MOUNTED_ATTR = "data-mounted";

/** {@link MOUNTED_ATTR} as a body selector, for `waitForSelector`. */
export const MOUNTED_SELECTOR = `body[${MOUNTED_ATTR}]`;

type MountRecorder = (payload: { href: string }) => void;

type GlobalWithMountRecorder = typeof globalThis & {
  __jaunderRecordMount?: MountRecorder;
};

/** Wait for the CSR mount and explicitly mark completion for OTEL capture. */
export async function waitForMount(
  page: Page,
  timeoutMs?: number,
): Promise<void> {
  await withTimedAction(page, "wait.mount", () =>
    page.waitForSelector(MOUNTED_SELECTOR, {
      timeout: timeoutMs,
    }),
  );

  await page.evaluate(() => {
    const globalScope = globalThis as GlobalWithMountRecorder;
    const recorder = globalScope.__jaunderRecordMount;
    if (typeof recorder === "function") {
      recorder({ href: location.href });
    }
  });
}
```

- [ ] **Step 2: Run the type checker, verify it fails**

Run: `devtool run -- cargo xtask check --no-test`

Expected: **FAIL** — `tsc` reports `Cannot find module './hydration'` from
`helpers.ts:40`, `helpers.ts:44`, and `layout-shift.ts:14`. This is the contract
for the rest of this task: every remaining step exists to drive it green.

- [ ] **Step 3: Update the Rust side**

In `csr/src/lib.rs`, replace lines 10–19 (the comment and the `inline_js` block)
with:

```rust
// The e2e suite waits on `body[data-mounted]` as the "app is mounted and
// interactive" signal — the counterpart of `MOUNTED_ATTR` in
// `end2end/tests/mount.ts`. The two literals must agree; if they drift, every
// e2e test times out.
#[wasm_bindgen::prelude::wasm_bindgen(inline_js = "
    export function mark_ready() {
        if (document && document.body) {
            document.body.setAttribute('data-mounted', 'true');
        }
    }
")]
```

Leave `extern "C" { fn mark_ready(); }` and everything below it unchanged.

- [ ] **Step 4: Update `fixtures.ts` — the Node-scope sites**

Add to the import block (alongside the existing `./helpers`, `./mail`,
`./selectors` imports at `:34-36`):

```ts
import { MOUNTED_ATTR, MOUNTED_SELECTOR } from "./mount";
```

Then apply, in order:

| Line   | From                                                                               | To                                                                                    |
| ------ | ---------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------- |
| `:3`   | `test in OTel capture: it instruments page requests, navigations, and hydration,`  | `test in OTel capture: it instruments page requests, navigations, and the CSR mount,` |
| `:70`  | `  hydratedMs: number \| null;`                                                    | `  mountedMs: number \| null;`                                                        |
| `:169` | `    await page.waitForSelector("body[data-hydrated]", {`                          | `    await page.waitForSelector(MOUNTED_SELECTOR, {`                                  |
| `:446` | `      await page.exposeBinding("__jaunderRecordHydration", (_source, value) => {` | `      await page.exposeBinding("__jaunderRecordMount", (_source, value) => {`        |
| `:456` | `          if (navigation.hydratedMs !== null) continue;`                          | `          if (navigation.mountedMs !== null) continue;`                              |
| `:458` | `          navigation.hydratedMs = nowMs;`                                         | `          navigation.mountedMs = nowMs;`                                             |
| `:543` | `            hydratedMs: null,`                                                    | `            mountedMs: null,`                                                        |
| `:722` | `            navigation.hydratedMs ??`                                             | `            navigation.mountedMs ??`                                                 |
| `:738` | `            navigation.committedMs !== null && navigation.hydratedMs !== null`    | `            navigation.committedMs !== null && navigation.mountedMs !== null`        |
| `:739` | `              ? navigation.hydratedMs - navigation.committedMs`                   | `              ? navigation.mountedMs - navigation.committedMs`                       |

And replace the `:452-453` comment:

```ts
// The mount-ready marker should be attributed to the most recent matching
// navigation (`data-mounted` is set once per document).
```

- [ ] **Step 5: Update `fixtures.ts` — the browser-scope init script**

**Read this step before editing.** The function at `:463` is passed to
`page.addInitScript()`, which **serializes it and runs it in the browser**. It
cannot reference `MOUNTED_ATTR` from Node module scope — doing so throws
`ReferenceError` in the page. Pass the constant as `addInitScript`'s second
argument instead.

Replace lines 463–505 with:

```ts
      await page.addInitScript((mountedAttr: string) => {
        const globalScope = globalThis as typeof globalThis & {
          __jaunderLongTasks?: Array<{
            startTime: number;
            duration: number;
            name: string;
          }>;
          __jaunderMountNotified?: boolean;
          __jaunderRecordMount?: (payload: { href: string }) => void;
        };
        globalScope.__jaunderLongTasks = [];
        globalScope.__jaunderMountNotified = false;

        const notifyMount = () => {
          if (globalScope.__jaunderMountNotified) return;
          const body = document.body;
          if (!body || !body.hasAttribute(mountedAttr)) return;
          globalScope.__jaunderMountNotified = true;
          try {
            globalScope.__jaunderRecordMount?.({ href: location.href });
          } catch {
            // Ignore cross-context bridge errors while collecting diagnostics.
          }
        };

        notifyMount();
        if (document.readyState === "loading") {
          document.addEventListener("DOMContentLoaded", notifyMount, {
            once: true,
          });
        }
        try {
          const mountObserver = new MutationObserver(() => notifyMount());
          mountObserver.observe(document.documentElement, {
            subtree: true,
            attributes: true,
            attributeFilter: [mountedAttr],
          });
        } catch {
          // MutationObserver should always exist in browsers, but keep this defensive.
        }
```

Everything from `:507`
(`if (typeof PerformanceObserver === "undefined") return;`) onward is unchanged
— but the `addInitScript` call's closing must now pass the argument. That
closing token is at **`:523`**. Change it to:

```ts
      }, MOUNTED_ATTR);
```

**Do not edit `:518`** — that is a decoy `});` closing the `PerformanceObserver`
callback inside the init script, not the `addInitScript` call. Editing it
produces a syntax error; editing neither leaves `mountedAttr` `undefined` in the
browser, which fails silently in a diagnostics-only path.

- [ ] **Step 6: Update the importers and their comments**

`end2end/tests/helpers.ts`:

- `:16-18` — replace the bullet with:
  ```
   * - `goto` waits for the CSR mount automatically.  Call
   *   `waitForMount(page)` only after action-triggered navigations (e.g.
   *   redirects from form submits, server-side 302s) where `goto` was not used.
  ```
- `:40` — `import { waitForHydration } from "./hydration";` →
  `import { waitForMount } from "./mount";`
- `:44` — `export { waitForHydration } from "./hydration";` →
  `export { waitForMount } from "./mount";`
- `:71` — `await waitForHydration(page, options?.timeout);` →
  `await waitForMount(page, options?.timeout);`

`end2end/tests/layout-shift.ts`:

- `:4` — `` * release, `body[data-hydrated]`, and `document.fonts.ready` `` →
  `` * release, `body[data-mounted]`, and `document.fonts.ready` ``
- `:14` — `import { waitForHydration } from "./hydration";` →
  `import { waitForMount } from "./mount";`
- `:32` — `   * hydration, before the after-sample.` →
  `   * the mount, before the after-sample.` (a straggler from an earlier
  partial migration: `:26` and `:30` in the same doc comment already say
  "mount")
- `:82` — `await waitForHydration(page);` → `await waitForMount(page);`

`end2end/tests/password_reset.spec.ts`:

- `:6` — `  waitForHydration,` → `  waitForMount,`
- `:41` —
  `// Login with new password should succeed from the same hydrated login page.`
  →
  `// Login with new password should succeed from the same mounted login page.`
- `:46` — `await waitForHydration(page);` → `await waitForMount(page);`

`end2end/tests/feeds.spec.ts`:

- `:2` —
  `import { goto, register, click, waitForHydration, BASE_URL } from "./helpers";`
  → `import { goto, register, click, waitForMount, BASE_URL } from "./helpers";`
- `:128` and `:200` — `await waitForHydration(page);` →
  `await waitForMount(page);`

`end2end/tests/auth.spec.ts:76-77`:

```ts
// No waitForMount: login is a client-side pushState now, so `data-mounted`
// (per-document) is already set — assert on content readiness instead (#591).
```

`end2end/tests/authed-cls.spec.ts:16`:

```
 * `body[data-mounted]`, never a timer) — safe under `workers>1` (#182).
```

`end2end/tests/timeline-cls.spec.ts`:

- `:9` —
  `* suite cannot check it: those tests assert content AFTER hydration settles and would`
  →
  `* suite cannot check it: those tests assert content AFTER the mount settles and would`
- `:19` —
  ``* `document.fonts.ready` + `body[data-hydrated]`, never a timer, so it is safe under``
  →
  ``* `document.fonts.ready` + `body[data-mounted]`, never a timer, so it is safe under``

- [ ] **Step 7: Run the type checker, verify it passes**

Run: `devtool run -- cargo xtask check --no-test`

Expected: **PASS** — `tsc`, `prettier`, `fmt`, and `clippy` all clean. If `tsc`
still reports an unresolved `./hydration`, a Step 6 importer was missed.

- [ ] **Step 8: Verify the acceptance criteria this task owns**

```bash
# AC2 — the marker moved
rg -F "data-mounted" csr/src/lib.rs
ls end2end/tests/mount.ts && ! ls end2end/tests/hydration.ts

# AC3.1 — exactly one definition of the name (zero matches is a FAIL)
rg -F '"data-mounted"' end2end/

# AC3.2 — no residual hardcoded selector literal
rg -F '[data-mounted]"' end2end/

# AC4 — the span renamed
rg -F '"wait.mount"' end2end/tests/mount.ts
```

Expected: `csr/src/lib.rs` matches; `mount.ts` exists and `hydration.ts` does
not; **AC3.1 matches exactly once, in `mount.ts`**; **AC3.2 returns no
matches**; the AC4 grep matches once.

Do **not** substitute a single unquoted `rg -F 'data-mounted' end2end/` here.
Step 6 deliberately writes the attribute name into prose comments in
`layout-shift.ts`, `authed-cls.spec.ts`, `timeline-cls.spec.ts` and
`auth.spec.ts`, so "matches only `mount.ts`" is unsatisfiable. Both greps above
are immune: those comments write the name in backticks, never in double quotes.

- [ ] **Step 9: Commit**

Run `devtool run -- cargo xtask check` first and confirm it is green
(**jaunder-commit**).

```bash
git add csr/src/lib.rs end2end/tests/mount.ts end2end/tests/fixtures.ts end2end/tests/helpers.ts end2end/tests/layout-shift.ts end2end/tests/password_reset.spec.ts end2end/tests/feeds.spec.ts end2end/tests/auth.spec.ts end2end/tests/authed-cls.spec.ts end2end/tests/timeline-cls.spec.ts
git commit -m "refactor(e2e): rename the data-hydrated mount marker to data-mounted (#251)"
```

---

### Task 2: Rewrite the concept-only prose

**Files:**

- Modify: `end2end/tests/atompub.spec.ts:87`
- Modify: `end2end/tests/media.spec.ts:94`
- Modify: `end2end/tests/posts.spec.ts:215`
- Modify: `end2end/tests/theme.spec.ts:5`, `:8`, `:11`
- Modify: `xtask/src/steps/build_csr.rs:5`
- Test: none. Comment-only, plus one test _title_ (D5). The contract is AC1's
  zero-residue grep in Step 3.

**Interfaces:**

- Consumes: nothing — no symbol from Task 1 is referenced by these edits.
- Produces: the `theme.spec.ts` test title
  `"issue #22: .j-root keeps a real data-theme after CSR mount"`, which **Task 4
  depends on** — the capture must observe the new title.

- [ ] **Step 1: Apply the five prose edits**

`end2end/tests/atompub.spec.ts:87`:

```ts
// goto waits for the CSR mount, so the label input is safe to fill.
```

`end2end/tests/media.spec.ts:94`:

```ts
// sibling media-page tests below — a bare `goto` races the CSR shell's mount.
```

`end2end/tests/posts.spec.ts:215`:

```ts
// refactored from a post-mount Effect into the Suspense block; must pass
```

`end2end/tests/theme.spec.ts` — `:5`, `:8`, `:11`:

```ts
// element must survive the CSR mount. A leaked Leptos `attr:` directive prefix
```

```ts
test("issue #22: .j-root keeps a real data-theme after CSR mount", async ({
```

```ts
await goto(page, "/"); // public projector home; goto() waits for the CSR mount
```

`xtask/src/steps/build_csr.rs:5`:

```rust
//! dev loop; the CSR mount is slower); `--release` matches CI's optimized wasm.
```

- [ ] **Step 2: Run the gate**

Run: `devtool run -- cargo xtask check --no-test`

Expected: **PASS**. (`prettier` may reflow a comment; it auto-fixes in `check`
mode — re-run once if it reports a fix.)

- [ ] **Step 3: Verify AC1 — zero residue**

```bash
git ls-files csr end2end xtask/src/steps/build_csr.rs | xargs rg -i hydrat
```

Expected: **no matches** (exit 1 from `rg`, which is the success condition
here). Any hit means a site was missed — fix it before committing. Note the
`git ls-files` scoping is deliberate: the untracked `end2end/CLAUDE.md` is not
gitignored and would otherwise produce a spurious hit until Task 5's checklist
item.

- [ ] **Step 4: Commit**

Run `devtool run -- cargo xtask check` first and confirm it is green
(**jaunder-commit**).

```bash
git add end2end/tests/atompub.spec.ts end2end/tests/media.spec.ts end2end/tests/posts.spec.ts end2end/tests/theme.spec.ts xtask/src/steps/build_csr.rs
git commit -m "docs(e2e): drop the remaining CSR-mount \"hydration\" prose (#251)"
```

---

### Task 3: Rename the OTel action in the docs

**Files:**

- Modify: `docs/observability.md:291-294`, `:456`
- Test: none. The contract is AC5's grep in Step 3.

**Interfaces:**

- Consumes: the span name `wait.mount` established in Task 1 (`mount.ts`).
- Produces: nothing consumed by later tasks.

- [ ] **Step 1: Rename in place inside the dated findings section**

Per D3 this follows #224's convention at `:306`/`:308` — new name inline, old
name deleted, a trailing note recording the rename. Replace the `:291-294`
bullet with:

```markdown
- The delta lives in **`navigation.commit_to_mount`** (the commit → CSR
  mount-ready phase): firefox 1123ms vs chromium 559ms = **2.01×**. The
  `wait.mount` action (the mount-ready wait) is the single largest action bucket
  (655ms avg × 302 = 198s); the action was renamed from `wait.hydration` in
  #251.
```

**Do not** touch `:267`, `:280-281`, `:300`, or `:308` in this section — those
sentences are about _real_ SSR hydration and its removal, and are correct as
written.

- [ ] **Step 2: Update the live warmup paragraph**

Replace `:455-457` with:

```markdown
This warmup runs on the same test page/context and waits for
`body[data-mounted]`, so subsequent navigations within that test are measured as
warm-cache behavior.
```

- [ ] **Step 3: Verify AC5**

```bash
rg -cF 'wait.hydration' docs/observability.md
rg -Uc 'renamed\s+from\s+`wait\.hydration`\s+in\s+#251' docs/observability.md
rg -F 'data-hydrated' docs/observability.md
```

Expected: the first two both report **exactly 1** (the same occurrence — D3's
rename note necessarily quotes the old span name), and the third returns **no
matches**.

The second command's `-U` (multiline) and the `\s+` at **every** word gap are
load-bearing: prettier reflows this note, and the wrap point moves as
surrounding text changes. A literal-space pattern reports zero even when the
text is correct — this bit twice during implementation, at two different wrap
points.

If the first reports more than 1, the `:291-294` bullet still carries the old
name outside the note.

- [ ] **Step 4: Commit**

Run `devtool run -- cargo xtask check` first and confirm it is green
(**jaunder-commit**).

```bash
git add docs/observability.md
git commit -m "docs(observability): rename the wait.hydration action to wait.mount (#251)"
```

---

### Task 4: Regenerate the server-fn coverage evidence

**Files:**

- Modify: `docs/coverage/server-fns-evidence.json` (generated — never
  hand-edited)
- Possibly modify: `docs/coverage/server-fns.json` (regenerated alongside;
  expected unchanged, since no server fn's coverage set changes)
- Test: none. The contract is AC6's grep in Step 3.

**Interfaces:**

- Consumes: the renamed test title from **Task 2** — this task is invalid if run
  before it, because the capture would record the old title.
- Produces: nothing consumed by later tasks.

- [ ] **Step 1: Produce a fresh capture**

Run (long — use the Bash tool's background mode):

```
devtool run -- cargo xtask e2e sqlite chromium
```

Expected: **PASS**. This also serves as an early read on AC7 for one of the four
combos — if the Rust/TS attribute pair drifted in Task 1, every test in this run
times out.

- [ ] **Step 2: Regenerate both artifacts**

Run: `devtool run -- cargo xtask server-fn-coverage regenerate`

Expected: **PASS**, rewriting `docs/coverage/server-fns.json` and
`docs/coverage/server-fns-evidence.json`.

- [ ] **Step 3: Verify AC6**

```bash
rg -cF 'after CSR mount' docs/coverage/server-fns-evidence.json
rg -cF 'after CSR hydration' docs/coverage/server-fns-evidence.json
```

Expected: the first reports **1 or more**; the second reports **no matches**.

Do not assert an exact count. The old title currently appears 4× (`:153`,
`:305`, `:849`, `:1037`), but that number is an artifact of which server fns the
theme test happened to drive in the recorded run — and #745 documents this
generator as run-to-run variable. "At least one, and none of the old" is what
AC6 actually requires.

- [ ] **Step 4: Commit**

Run `devtool run -- cargo xtask check` first and confirm it is green
(**jaunder-commit**).

Expect a large diff: per the #745 notes a regenerate churns ~66 unrelated
evidence entries. That is accepted under D5 — say so in the commit body so a
reviewer is not surprised.

```bash
git add docs/coverage/server-fns.json docs/coverage/server-fns-evidence.json
git commit -m "chore(coverage): regenerate server-fn evidence for the renamed theme test (#251)

The theme spec's title changed with the mount-marker rename. Regenerating the
evidence also rewrites ~66 unrelated entries, which is inherent to the
generator (#745) and expected in this diff."
```

---

### Task 5: Full gate and ship checklist

**Files:** none modified in the branch.

**Interfaces:**

- Consumes: all four preceding tasks.
- Produces: the AC7 green that authorizes shipping.

- [ ] **Step 1: Run the full local gate**

Run (long — use the Bash tool's background mode):

```
devtool run -- cargo xtask validate
```

Expected: **PASS**, including all four `{sqlite,postgres}×{chromium,firefox}`
e2e combos. This is AC7 and the real proof of the rename: a Rust/TS attribute
mismatch times out every test rather than degrading quietly.

- [ ] **Step 2: Re-verify AC1 on the final tree**

```bash
git ls-files csr end2end xtask/src/steps/build_csr.rs | xargs rg -i hydrat
```

Expected: **no matches**.

- [ ] **Step 3: Ship checklist — update the untracked agent doc**

Per D7 this file is **untracked and stays untracked**: edit it in the **main
checkout** (`/home/mdorman/src/jaunder/end2end/CLAUDE.md`), never `git add` it,
and do not commit it on this branch.

The file has **10** hydration references on 9 lines. Work from a fresh grep, not
from memory:

```bash
rg -n 'hydrat' /home/mdorman/src/jaunder/end2end/CLAUDE.md   # expect 9 lines
```

Two independent fixes:

1. **This cycle's rename** — `:12`, `:26`, `:27`, `:31`, `:32`, `:40`, `:50`,
   `:53`: `waitForHydration` → `waitForMount`, `body[data-hydrated]` →
   `body[data-mounted]`, and "(Leptos WASM) hydration" → "the CSR mount". Note
   `:26-27` and `:31-32` are prose sentences that span two lines each
   ("`prop:value` bindings reset input values during WASM hydration; filling
   before hydration completes sends empty fields to the server") — reword the
   sentence, don't swap a single word.
2. **Pre-existing #224 debt** — `:15`, `:18`: `hydrationHeavyTimeoutMs` →
   `slowBrowserTimeoutMs` and `hydrationHeavyFirstNavigationTimeoutMs` →
   `slowBrowserFirstNavigationTimeoutMs`. These helpers were renamed by commit
   `4b5b2ac4` and the doc never caught up; verify the current names against
   `end2end/tests/fixtures.ts:187` and `:198` before writing.

Verify by re-running the grep above: expect **no matches**.

Nothing tracks or verifies this step (spec, "Ship checklist"). If it is skipped,
say so explicitly at the ship gate rather than letting it pass silently.

- [ ] **Step 4: Hand off to `jaunder-ship`**

No commit in this task. Proceed to **jaunder-ship** for the final review,
plan/spec archiving, push, and PR.
