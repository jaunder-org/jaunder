# #308 — visual regression testing for core CSR states

Issue: [#308](https://github.com/jaunder-org/jaunder/issues/308). Milestone:
Test infrastructure & E2E. The issue has no blockers.

## Problem

The Playwright suite proves behavior across the four
`{sqlite,postgres}×{chromium,firefox}` combinations, but it does not compare the
rendered result. A change can preserve selectors and interactions while breaking
the desktop shell, page composition, typography, spacing, or Post-card layout.
The CSR surface contributes no host coverage, so this is a material blind spot.

Naively adding `toHaveScreenshot` to ordinary tests is not deterministic. The
suite is fully parallel and several tests publish Posts into the global public
timeline. The Studio theme also prefers ambient `Geist`/`Inter` fonts that the
current Nix environments do not pin. Exact committed images therefore require
both state ordering and a controlled screenshot font environment.

## Decisions

| ID      | Decision                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| ------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **D1**  | Add one viewport screenshot assertion to each of four existing behavioral tests: the public timeline/default theme in `theme.spec.ts`, the login form in `auth.spec.ts`, the authenticated `/app` cockpit and inline composer in `authed-flash.spec.ts`, and the empty `/posts/new` editor in `posts.spec.ts`. Do not create a visual-only spec. Each test carries Playwright's native `@visual` tag, so the distributed inventory is searchable, report-visible, and runnable with Playwright's normal tag filter.                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| **D2**  | Compare the complete existing desktop viewport. Keep the current Desktop Chrome and Desktop Firefox device definitions; do not add a viewport matrix. Baselines are browser-specific and Linux-specific through Playwright's project/platform naming. Chromium and Firefox each have one expected image per state. WebKit does not run the tagged tests.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| **D3**  | Exact means zero tolerated pixel or color difference. A shared visual assertion helper owns `toHaveScreenshot` with a zero color threshold and the screenshot-only stabilization policy. Playwright disables animation and hides carets during comparison. The public-timeline assertion masks only the generated publication-time text; stable shell, Post count/order, author, avatar, title, body, spacing, and geometry remain compared. No broad region or dynamic-layout masking is permitted.                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| **D4**  | Add `chromium-visual` and `firefox-visual` prerequisite projects to the single shared `end2end/playwright.config.ts`. Each selects `@visual`, sets `retries: 0`, and runs before mutation-heavy tests; the ordinary browser project excludes `@visual` and depends on its visual project; the existing serial admin project continues to depend on the ordinary project. Thus each tagged behavioral test runs exactly once against the canonical fresh database, and an exact visual failure never retries state-producing setup in the same database. This extends ADR-0051's one-config design rather than creating a second config or CI lane.                                                                                                                                                                                                                                                                                                                  |
| **D5**  | The public-timeline behavioral test creates one stable `visualauthor` and one fixed-content public Post, then waits for that Post before comparing. Its generated publication-time label is the sole masked value. The two authenticated visual states use the canonical read-only `testlogin` identity, avoiding generated usernames without introducing shared writes. The visual project runs before ordinary tests, and no other visual state depends on the public timeline, so the Post cannot race or leak into another expected state.                                                                                                                                                                                                                                                                                                                                                                                                                      |
| **D6**  | Pin screenshot typography without changing production CSS. Nix provides one explicit DejaVu font set and fontconfig file to both the host Playwright process and the NixOS-VM Playwright process. A screenshot-only stylesheet overrides Jaunder's body/display/meta/mono font variables with those pinned families. Browser version, viewport, font files, and font selection are therefore common between baseline production and CI comparison.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| **D7**  | Extend `cargo xtask e2e-local` with one narrow `--update-visual-snapshots` mode. It builds release CSR, runs only the two visual projects with Playwright snapshot-update mode, and gives Chromium and Firefox separate fresh server/database lifecycles so each baseline is produced from the same state its CI VM sees. It retains e2e-local's lifecycle ownership, teardown, and zero-panic verification. The ordinary Chromium debug loop remains unchanged. To preserve the existing positional-filter contract, a filtered normal run first invokes the matching `chromium-visual` test with `--no-deps --pass-with-no-tests`, then invokes the filtered ordinary/admin projects with the same two flags; the pass-through is required because a spec such as `theme.spec.ts` may contain only a tagged test. An unrelated spec does not pull all four dependency tests into the tight loop. The update flag is not combinable with a positional test filter. |
| **D8**  | Commit Playwright's expected PNGs beside their owning specs under the default `*-snapshots/` directories. The same browser image is used by SQLite and PostgreSQL; a backend-dependent rendering difference must fail rather than gain a second expected image. CI continues invoking the existing four e2e derivations and workflows. Project dependencies pull in the visual project, so no GitHub Actions implementation or matrix row is added.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| **D9**  | Document the baseline lifecycle in `CONTRIBUTING.md`: run the one xtask update command, inspect every changed PNG, run the tagged comparisons without update mode, and commit intentional images with the code that changed rendering. Update `docs/ARCHITECTURE.md` with the visual→main→admin project chain, shared-backend baseline rule, pinned screenshot font environment, and CI path.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| **D10** | No ADR or domain-model change is needed. The work applies Playwright's established visual-comparison mechanism inside ADR-0051's shared configuration and ADR-0039's project-dependency isolation. The baseline set and updater are reversible testing policy, recorded in the contributor guide and architecture view rather than a new architectural decision.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |

## Observable flow

1. A normal `cargo xtask e2e-local`, `cargo xtask e2e <backend> <browser>`, or
   full validation selects the ordinary browser and admin projects.
2. Playwright first runs that browser's prerequisite visual project against the
   freshly seeded database. Only the four `@visual` behavioral tests run there.
3. The public-timeline test creates its fixed visual Post; every test waits for
   its existing behavioral readiness condition before invoking the shared
   viewport comparison.
4. Playwright compares against the committed browser/Linux PNG using the pinned
   fontconfig and screenshot stylesheet. Any changed unmasked pixel fails.
5. After the visual prerequisite succeeds, the ordinary fully parallel project
   and then the existing serial admin project run as before.
6. For an intentional rendering change, a contributor runs
   `cargo xtask e2e-local --update-visual-snapshots`. Xtask produces Chromium
   and Firefox images from separate fresh release-mode lifecycles, then the
   contributor reviews and commits the PNG changes.

## Acceptance criteria

- **AC1 — four existing behavioral contracts.** Exactly four existing tests are
  tagged `@visual` and compare the complete desktop viewport: public timeline,
  login form, authenticated `/app` cockpit/inline composer, and empty
  `/posts/new` editor. No visual-only spec duplicates their setup or assertions.

- **AC2 — exact browser baselines.** Chromium and Firefox each have a committed
  Linux PNG for every visual state. Comparison allows no changed pixel and uses
  a zero color threshold. Only the public Post's generated time text is masked;
  no shell, route region, Post body, form, or editor region is masked.

- **AC3 — deterministic state ordering.** `chromium-visual` and `firefox-visual`
  select `@visual` and set `retries: 0`; their ordinary sibling projects exclude
  the tag and depend on them. The existing admin projects remain downstream of
  the ordinary projects. In a full run, tagged tests therefore execute once,
  before the parallel mutation-heavy suite, against a fresh canonical seed.

- **AC4 — stable state content.** The public visual test waits for one
  fixed-content Post by a stable `visualauthor`. The authenticated visual tests
  render as stable `testlogin` and do not persist a mutation. Repeating either
  browser's visual project from a fresh database produces byte-identical
  expected images. A visual comparison failure produces one attempt and cannot
  retry its fixed state-producing setup in the same database.

- **AC5 — pinned typography.** Host baseline generation and all NixOS-VM
  comparisons receive the same Nix-store DejaVu font files and fontconfig. The
  screenshot-only stylesheet selects those families without changing the
  application's production stylesheet or runtime rendering.

- **AC6 — one safe local interface.** A positional
  `cargo xtask e2e-local <spec>` run executes only a matching tagged visual
  test, if any, followed by that spec's ordinary/admin tests; either empty
  selection passes, and Playwright's dependency expansion does not schedule the
  other visual tests. `cargo xtask e2e-local --update-visual-snapshots` needs no
  pre-existing server or database, uses release CSR, updates only `@visual`
  baselines for both gated browsers, gives each browser a fresh lifecycle, tears
  each server down, and applies the normal diagnostic/zero-panic checks.
  Combining the mode with a positional test filter fails with an actionable
  argument error.

- **AC7 — existing CI matrix.** The four existing
  `{sqlite,postgres}×{chromium,firefox}` e2e checks execute their browser's
  visual prerequisite and compare the shared browser baseline. No new workflow,
  backend-specific expected image, WebKit baseline, or matrix row is introduced.

- **AC8 — documented review workflow.** `CONTRIBUTING.md` names the visual-test
  tag and four-state policy, baseline location, exact comparison rule, update
  command, required image review, and targeted verification command.
  `docs/ARCHITECTURE.md` records the project chain, pinned font environment,
  backend sharing, and existing CI path.

- **AC9 — regression proof.** Xtask tests pin normal, positional-filter, and
  visual-update command planning, including `--no-deps` and
  `--pass-with-no-tests` on both filtered invocations, plus invalid option
  combinations. A filtered unrelated spec does not schedule the four visual
  tests. Configuration-focused evidence pins `retries: 0` for both visual
  projects, and a forced visual failure records one attempt. Type checking
  accepts the tagged tests and project graph. A clean visual run passes for
  Chromium and Firefox; changing an expected image makes the corresponding
  comparison fail. The full repository shipping gate passes with all four
  backend/browser combinations.

## Non-goals

- Mobile, tablet, responsive-breakpoint, WebKit, dark-mode, or alternate-theme
  baselines.
- Snapshotting every route, transient state, validation error, or component.
- Replacing behavioral assertions, host tests, wasm browser unit tests, or
  Playwright trace/screenshot-on-failure diagnostics.
- Backend-specific visual expectations or a separate visual CI workflow.
- Updating production typography, bundling production web fonts, or declaring
  the pinned screenshot font to be the product's font contract.
- Pixel-diff approval automation or storing binary baselines outside Git.

## Risks

- **Baseline churn.** Exact screenshots deliberately turn every rendered pixel
  into review surface. Four states and one desktop viewport bound the initial
  cost; intentional changes update images in the same commit.
- **Hidden nondeterminism.** Global Posts, generated usernames, publication
  times, animation, carets, and ambient fonts can all perturb images. The visual
  prerequisite, fixed identities/content, one narrow time mask, Playwright's
  screenshot stabilization, and pinned fontconfig address each known source.
- **Project-graph mistakes.** A tagged test that also runs in the ordinary
  project would mutate twice or use a differently named baseline. Reciprocal
  `grep`/`grepInvert` plus dependency-focused tests must pin run-once ordering.
- **Gate cost.** The four tests already exist and move from the main project
  rather than duplicating there, but each browser gains one prerequisite-project
  startup and screenshot comparison. Keeping the initial set at four avoids a
  second suite hidden inside the suite.
- **Updater/gate drift.** A host debug build, shared database across browser
  updates, or ambient fontconfig could generate images CI does not reproduce.
  The single update mode owns release selection, per-browser fresh lifecycles,
  and the same Nix font configuration used by the VM.
