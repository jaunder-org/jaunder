# Visual Snapshot Regression Testing — Implementation Plan

> **For agentic workers:** Execute with `jaunder-iterate`; use
> `jaunder-dispatch` only for a self-contained task where delegation helps. Tick
> each checkbox when its step is complete.

**Goal:** Add exact, deterministic Chromium and Firefox viewport baselines to
four existing Playwright behavioral tests, with one safe local update command
and no new CI lane.

**Architecture:** Extend ADR-0051's single Playwright configuration with
per-browser visual prerequisite projects. A small visual assertion module owns
exact comparison and screenshot-only typography; Nix supplies the same
DejaVu/fontconfig environment to host and VM Playwright. Refactor `e2e-local`
around a pure invocation plan and a reusable fresh-server lifecycle so normal,
filtered, and two-browser update modes cannot drift.

**Tech stack:** TypeScript, Playwright 1.58.2, Rust/Clap/xshell xtask, NixOS VM
checks, PNG baselines.

**Issue:** [#308](https://github.com/jaunder-org/jaunder/issues/308)

**Approved spec:**
[`../specs/2026-08-13-issue-308-visual-snapshot-testing.md`](../specs/2026-08-13-issue-308-visual-snapshot-testing.md)

## Global constraints

- Preserve ADR-0051: one shared `end2end/playwright.config.ts` for host and CI.
- Preserve ADR-0039's ordering: visual → ordinary → serial admin for each
  browser.
- Exactly four existing behavioral tests carry `@visual`; do not create a
  visual-only spec.
- Chromium and Firefox only; desktop devices only; no WebKit, backend,
  responsive, or theme matrix.
- Exact comparison: `threshold: 0`, `maxDiffPixels: 0`; animations disabled and
  carets hidden. Only `.j-post-time` in the public timeline may be masked.
- Expected PNGs use Playwright's default spec-adjacent `*-snapshots/` layout.
  Never add backend-specific baselines.
- Visual projects set `retries: 0`, even when CI exports
  `JAUNDER_E2E_RETRIES=1`.
- Update mode uses release CSR and separate fresh server/database lifecycles for
  Chromium and Firefox. Every lifecycle tears down and runs the shared
  zero-panic verifier.
- A filtered normal run bypasses dependency expansion: matching visual test
  first with `--no-deps --pass-with-no-tests`, then matching ordinary/admin
  tests with the same two flags. Either selection may be empty; an unrelated
  filter must not run all visual tests.
- Nix supplies one explicit DejaVu/fontconfig environment to both host and VM
  browser processes. Production CSS remains unchanged.
- Run commands through `devtool run --`. Invoke pinned tools directly; never use
  `npx`, npm scripts, or `nix develop -c`.
- Before every kept commit, run `devtool run -- cargo xtask check`, stage the
  exact intended tree, and commit without a `Co-Authored-By` trailer.

---

## Task 1: Add the visual project and typography contracts

**Files:**

- Create: `end2end/tests/visual.ts`
- Create: `end2end/tests/visual.css`
- Modify: `end2end/playwright.config.ts`
- Modify: `flake.nix`

**Interfaces:**

- `expectVisual(page: Page, name: string, options?: { mask?: Locator[] }): Promise<void>`
  - Applies `visual.css` only during capture through Playwright's screenshot
    `stylePath` option.
  - Waits for `document.fonts.ready` before comparing.
  - Calls
    `expect(page).toHaveScreenshot(name, { animations: "disabled", caret: "hide", threshold: 0, maxDiffPixels: 0, mask, stylePath })`.
  - Captures the current viewport; does not use `fullPage`.
- `visual.css`
  - Overrides Jaunder's body/display/meta/mono font variables with DejaVu Sans
    and DejaVu Sans Mono only during comparison.
  - Does not duplicate layout, color, or component styling.
- Playwright project graph per browser:
  - `<browser>-visual`: native desktop device, `grep: /@visual/`, `retries: 0`.
  - `<browser>`: same device, `grepInvert: /@visual/`,
    `dependencies: ["<browser>-visual"]`.
  - `<browser>-admin`: unchanged admin match/serialization, now downstream of
    `<browser>`.
  - `webkit`: `grepInvert: /@visual/`; no visual dependency or baseline.
- Nix font contract:
  - One `visualFontConfig` made from `pkgs.dejavu_fonts`.
  - `FONTCONFIG_FILE` points at it in both dev shells' shared `shellEnv` and in
    the VM Playwright command assembled by `e2eRunAndCapture`.
  - DejaVu is in the closure used by host baseline generation and both backend
    VMs; no ambient host font lookup remains.

- [x] **Step 1: Create the exact visual assertion helper and stylesheet**

Implement `visual.ts` and `visual.css` with the interface above. Resolve
`stylePath` relative to the helper module; do not inject permanent page styles.
Keep the timestamp mask caller-owned because only one state needs it.

- [x] **Step 2: Extend the Playwright project graph**

Add `chromium-visual` before `chromium`, and `firefox-visual` before `firefox`.
Preserve existing device and launch options. Exclude `@visual` from ordinary and
WebKit projects; keep admin matching and serialization unchanged. Set
`retries: 0` directly on each visual project.

- [x] **Step 3: Pin the host and VM font environment**

Define one DejaVu-only fontconfig value in the per-system Nix scope. Export it
as `FONTCONFIG_FILE` from the shared shell environment and prepend it to the
VM's Playwright process environment. Ensure the referenced font derivation is
retained by the relevant closures rather than relying on
`/run/current-system/sw/share/fonts`.

- [x] **Step 4: Type-check and inspect the configuration**

Run:

```bash
devtool run -- tsc --noEmit -p end2end/tsconfig.json
devtool run -- playwright test --config end2end/playwright.config.ts --list
```

Expected: both pass, proving the configuration loads before tagged tests exist.
The ordinary projects list their current tests; zero-test visual projects need
not emit list entries. Task 4 proves the resolved seven-project graph after
tagging.

- [x] **Step 5: Gate, stage, and commit Task 1**

Run `devtool run -- cargo xtask check`. Stage the four Task 1 implementation
files plus the approved issue #308 specification and this plan. Commit:

```text
test(e2e): add deterministic visual project harness (#308)
```

---

## Task 2: Preserve the filtered local loop across project dependencies

**Files:**

- Modify: `xtask/src/steps/e2e_local.rs`

**Interfaces:**

- Introduce a private, pure Playwright-invocation planner used by the existing
  `run` path. Each planned invocation owns its browser projects, filter, and
  dependency/pass-through flags.
- Normal unfiltered: one invocation selecting `chromium` + `chromium-admin`;
  dependency expansion schedules `chromium-visual`.
- Normal filtered: filtered `chromium-visual --no-deps --pass-with-no-tests`,
  then filtered `chromium chromium-admin --no-deps --pass-with-no-tests`.
- The existing one-server/one-database lifecycle executes the planned
  invocations in order. A real visual failure stops the downstream invocation;
  an empty selection succeeds and continues.
- The public CLI remains `cargo xtask e2e-local [<spec-or-file:line>]`; this
  task does not expose update mode.

- [x] **Step 1: Add failing pure-plan tests**

Add unit tests for the unfiltered and filtered plans. Assert complete ordered
argument vectors, including:

- unfiltered ordinary/admin projects with dependency expansion left enabled;
- filtered visual invocation before filtered ordinary/admin;
- `--no-deps` and `--pass-with-no-tests` on both filtered invocations;
- the same positional filter on both invocations.

Run:

```bash
devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml e2e_local
```

Expected: FAIL because the pure planner does not exist.

- [x] **Step 2: Implement and consume the normal-mode plan**

Move Playwright argument construction out of the lifecycle body. Execute each
planned invocation against the existing base URL, DB, capture directory, worker
setting, and PATH. Record which invocation failed. Stop after a real visual
failure, but retain unconditional server teardown and zero-panic verification.

Run the Step 1 command again. Expected: PASS.

- [x] **Step 3: Exercise both public normal paths**

Run:

```bash
devtool run -- cargo xtask e2e-local auth.spec.ts
```

Expected: PASS. At this point no tests are tagged yet, so the filtered visual
invocation selects nothing and passes; the filtered ordinary invocation runs
`auth.spec.ts`. The unfiltered path remains pinned by the pure-plan test and
will be exercised by the full browser gates after Task 4.

- [x] **Step 4: Gate, stage, and commit Task 2**

Run `devtool run -- cargo xtask check`. Stage `xtask/src/steps/e2e_local.rs` and
this plan. Commit:

```text
refactor(xtask): preserve filtered e2e project scope (#308)
```

---

## Task 3: Add complete visual snapshot update lifecycles

**Files:**

- Modify: `xtask/src/lib.rs`
- Modify: `xtask/src/steps/e2e_local.rs`

**Interfaces:**

- `cargo xtask e2e-local [<spec-or-file:line>] [--update-visual-snapshots]`.
- Clap rejects a positional filter combined with `--update-visual-snapshots`,
  and the diagnostic names the conflict.
- Extend the pure plan with:
  - whether CSR is release-built;
  - one or two fresh lifecycles;
  - ordered Playwright invocations within each lifecycle;
  - snapshot-update mode and browser identity.
- Normal plans remain byte-for-byte the Task 2 plans.
- Update plan: release CSR, Chromium visual update in one lifecycle, Firefox
  visual update in a second lifecycle; each selects only its visual project with
  `--no-deps` and Playwright's snapshot-update argument.
- Build phase executes once per command. Each planned lifecycle owns a new temp
  storage directory, SQLite DB, capture directory, runtime file, server-stderr
  log, ephemeral server, canonical seed, Playwright invocation, teardown, and
  zero-panic verification.
- Step results identify Chromium versus Firefox. Duplicate anonymous result rows
  are not acceptable.

- [x] **Step 1: Add failing CLI and update-plan tests**

Extend CLI parsing coverage with:

- default `update_visual_snapshots == false`;
- successful `e2e-local --update-visual-snapshots`;
- rejection of `e2e-local theme.spec.ts --update-visual-snapshots`.

Extend the pure-plan tests with:

- release CSR selection;
- Chromium-then-Firefox lifecycle order;
- separate lifecycle identities;
- visual-only project selection, `--no-deps`, and snapshot-update argument;
- unchanged normal/filtered plans.

Run:

```bash
devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml e2e_local
```

Expected: FAIL because the flag and update plan do not exist.

- [x] **Step 2: Add the flag and complete planner**

Add the boolean field to `Command::E2eLocal`, retain the optional positional
filter, and use Clap's argument conflict rather than a runtime special case.
Pass both values into `steps::e2e_local::run`. Extend the existing pure plan; do
not add a second command-construction path.

- [x] **Step 3: Separate build-once from run-once lifecycle ownership**

Build CSR according to the plan's release bit, and build the server plus
`test-support` once. Move temp storage, server startup, seed, Playwright, stop,
and panic verification into one lifecycle function. Loop over the planned
lifecycles. Keep `ServerChild` as the teardown guard on every early return.
Preserve the canonical DB/capture/PATH/base-URL environment and set
`PLAYWRIGHT_HTML_OPEN=never`.

- [x] **Step 4: Run regression and invalid-interface checks**

Run:

```bash
devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml e2e_local
devtool run -- cargo xtask e2e-local theme.spec.ts --update-visual-snapshots
```

Expected: unit tests PASS. The invalid CLI combination exits non-zero before any
build or server startup and reports the conflict. Do not run update mode
successfully yet; Task 4 supplies the four tagged contracts and baselines that
make its observable output meaningful.

- [x] **Step 5: Gate, stage, and commit Task 3**

Run `devtool run -- cargo xtask check`. Stage `xtask/src/lib.rs`,
`xtask/src/steps/e2e_local.rs`, and this plan. Commit:

```text
feat(xtask): update visual snapshots in fresh browsers (#308)
```

---

## Task 4: Add four visual behavioral contracts and baselines

**Files:**

- Modify: `end2end/tests/theme.spec.ts`
- Modify: `end2end/tests/auth.spec.ts`
- Modify: `end2end/tests/authed-flash.spec.ts`
- Modify: `end2end/tests/posts.spec.ts`
- Create: Playwright-generated PNGs under the four adjacent
  `*.spec.ts-snapshots/` directories

**Interfaces:**

- Every selected existing `test(...)` receives `{ tag: "@visual" }` and one
  `await expectVisual(...)` after its existing readiness assertions.
- Public timeline/default theme (`theme.spec.ts`):
  - create `visualauthor` with `seedUserViaTool` in the fresh database;
  - seed exactly one published Post with fixed title/body via
    `seedPostsViaTool`;
  - visit `/`, retain the existing theme assertions, and wait for the fixed
    Post/author;
  - compare the viewport with only `page.locator(".j-post-time")` masked.
- Login (`auth.spec.ts`): tag the existing `login page shows form` test and
  compare after heading/username/password visibility.
- Authenticated `/app` (`authed-flash.spec.ts`): adapt the existing cockpit test
  to apply a session for canonical `testlogin`, cold-navigate to `/app`, retain
  its feed/composer readiness assertions, and compare without writes.
- Empty `/posts/new` (`posts.spec.ts`): adapt the existing
  empty-body/disabled-controls test to canonical `testlogin`, cold-navigate to
  `/posts/new`, compare while still empty after its disabled-button assertions,
  then retain the whitespace/valid-body behavioral assertions.
- Snapshot names are stable semantic names, one per state. Let Playwright append
  project/platform identity; do not encode storage backend.

- [x] **Step 1: Add the four tagged comparisons**

Import only the needed visual/seed/session helpers. Do not create parallel
visual setup paths. Ensure the screenshot is taken after the behavioral state is
proven, before the empty-editor test mutates its textarea.

- [x] **Step 2: Type-check and prove the resolved project graph**

Run:

```bash
devtool run -- tsc --noEmit -p end2end/tsconfig.json
devtool run -- playwright test --config end2end/playwright.config.ts --list --project chromium-visual
devtool run -- playwright test --config end2end/playwright.config.ts --list --project chromium
devtool run -- playwright test --config end2end/playwright.config.ts --list --project chromium-admin
devtool run -- playwright test --config end2end/playwright.config.ts --list --project firefox-visual
devtool run -- playwright test --config end2end/playwright.config.ts --list --project firefox
devtool run -- playwright test --config end2end/playwright.config.ts --list --project firefox-admin
devtool run -- playwright test --config end2end/playwright.config.ts --list --project webkit
```

Expected: typecheck PASS. Each visual project lists exactly the same four
existing tests and no others. Selecting an ordinary project lists those four
under its visual dependency and every remaining non-admin test only under the
ordinary project. Selecting an admin project shows the complete visual →
ordinary → admin chain. WebKit omits all four tagged tests.

Run the resolved retry inventory with an explicit reporter output:

```bash
JAUNDER_E2E_RETRIES=1 PLAYWRIGHT_JSON_OUTPUT_FILE=/tmp/issue-308-playwright-projects.json devtool run -- playwright test --config end2end/playwright.config.ts --list --reporter=json
devtool run -- jq -e '([.config.projects[] | select(.name == "chromium-visual" or .name == "firefox-visual") | .retries] == [0, 0]) and ([.config.projects[] | select(.name == "chromium" or .name == "firefox") | .retries] == [1, 1])' /tmp/issue-308-playwright-projects.json
```

Expected: both pass. The exact `jq` predicate proves both visual overrides
resolve to zero while both ordinary siblings inherit the ambient retry value.

- [x] **Step 3: Generate both browser baseline sets**

Run:

```bash
devtool run -- cargo xtask e2e-local --update-visual-snapshots
```

Expected: PASS with release CSR. Chromium and Firefox each run against a
separately logged fresh lifecycle; both Playwright and panic-gate results pass.
Exactly eight PNGs are created: four states × two browser projects.

- [x] **Step 4: Prove fresh-run byte stability**

Record SHA-256 hashes for all eight PNGs, run the Step 3 command again, and
compare hashes.

Expected: all eight hashes are unchanged. Any changed image is a determinism
defect; fix the state/font/capture seam rather than tolerating pixels or adding
masks.

- [x] **Step 5: Prove exact failure and one-attempt behavior**

Temporarily replace the expected Chromium login PNG with a different valid state
PNG, preserving the original for restoration. Run
`cargo xtask e2e-local auth.spec.ts` through `devtool run` with
`JAUNDER_E2E_RETRIES=1` in the command environment. Inspect the parked
Playwright output/report.

Expected: non-zero visual comparison; the login test has exactly one attempt
despite the ambient retry setting. No other visual state is scheduled by the
positional filter. Restore the exact original PNG, rerun the same filtered
command under the same retry environment, and require PASS.

- [x] **Step 6: Prove both browser comparisons in the VM path**

Run:

```bash
devtool run -- cargo xtask e2e sqlite chromium
devtool run -- cargo xtask e2e sqlite firefox
```

Expected: both pass, each running its visual prerequisite before ordinary/admin
tests against the committed shared baseline. The Firefox run must not update
snapshots.

- [x] **Step 7: Gate, inspect images, stage, and commit Task 4**

Open or otherwise inspect every generated PNG; confirm the intended complete
desktop state, no clipping, correct pinned typography, and that only the
timestamp is masked in the public timeline. Run
`devtool run -- cargo xtask check`. Stage the four specs, all eight PNGs, and
this plan. Commit:

```text
test(e2e): baseline four visual browser states (#308)
```

---

## Task 5: Document the workflow and run the shipping gate

**Files:**

- Modify: `CONTRIBUTING.md`
- Modify: `docs/ARCHITECTURE.md`
- Modify:
  `docs/superpowers/plans/2026-08-13-issue-308-visual-snapshot-testing.md`

**Interfaces:**

- Contributor guide records:
  - `@visual` and the four-state policy;
  - spec-adjacent browser/Linux PNG locations;
  - exact zero-pixel comparison and sole timestamp mask;
  - `cargo xtask e2e-local --update-visual-snapshots` as the only supported
    updater;
  - review every changed PNG, run comparisons without update mode, and commit
    intentional baselines with rendering code;
  - targeted filtered command behavior.
- Architecture view records:
  - visual → ordinary → admin graph for Chromium and Firefox;
  - visual projects' zero-retry/fresh-state role;
  - one browser baseline shared by SQLite and PostgreSQL;
  - DejaVu/fontconfig plus screenshot-only CSS seam;
  - existing four-combination Nix/CI path, with no new workflow lane.

- [ ] **Step 1: Update the contributor testing guide**

Place the workflow beside the existing end-to-end testing guidance. Use commands
callable from the repository root and explicitly prohibit hand-running
Playwright snapshot update mode against an ambient server/database.

- [ ] **Step 2: Update the architecture materialized view**

Update the existing Playwright/e2e section rather than adding a disconnected
testing appendix. Describe current structure in present tense; cite ADR-0051 and
ADR-0039 without changing their historical text.

- [ ] **Step 3: Format and run the full shipping gate**

Run:

```bash
devtool run -- prettier -w CONTRIBUTING.md docs/ARCHITECTURE.md docs/superpowers/plans/2026-08-13-issue-308-visual-snapshot-testing.md
devtool run -- cargo xtask validate
```

Expected: full validation passes, including all four
`{sqlite,postgres}×{chromium,firefox}` e2e combinations. Each combination runs
the appropriate visual prerequisite against the shared browser baseline; no
snapshot is updated.

- [ ] **Step 4: Stage and commit Task 5**

Tick all completed plan checkboxes, then stage `CONTRIBUTING.md`,
`docs/ARCHITECTURE.md`, and this plan. Because the kept commit must match the
checked tree, rerun `devtool run -- cargo xtask check` after the checkbox edit.
Commit:

```text
docs(testing): document visual baseline workflow (#308)
```

- [ ] **Step 5: Confirm clean completion state**

Verify the working tree is clean. Confirm:

- exactly four tagged behavioral tests;
- exactly eight expected PNGs;
- no backend/WebKit/mobile baselines;
- update mode and positional filter remain mutually exclusive;
- the full gate's last result is successful.
