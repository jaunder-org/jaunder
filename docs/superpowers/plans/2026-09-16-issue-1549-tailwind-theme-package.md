# Tailwind Theme Package Proving Ground Implementation Outline

> Execute with `jaunder-iterate`, using `jaunder-dispatch` for bounded tasks.
> This outline exists because the work crosses repository and release
> boundaries, may expose public Style Contract changes, and requires immutable
> contracts between Jaunder and an external Theme repository.

## Scope

In:

- Bootstrap the public `jaunder-org/theme-tailwind` repository.
- Port the approved visual surface with a pinned Tailwind authoring toolchain.
- Prove Jaunder's canonical validation, artifact, release, and Studio lifecycle.
- Correct only Jaunder blockers demonstrated by the real package.
- Land, release, install, and visually verify the package in dependency order.

Out:

- Hugo application features, executable Theme behavior, or unsupported assets.
- A registry, gallery, remote installer, or speculative Style Contract growth.
- A promise of ongoing source compatibility with the upstream Hugo repository.

## Blocker loop

`PROVING-GROUND.md` is the issue-linked log across every task below. A task is
not complete while its proof has an unresolved failure. Each failure records a
reproduction and classification. A Jaunder defect receives focused red/green
proof, applicable backend parity and architecture documentation, then the
applicable broad verification-ladder stages; its Jaunder PR references
`jaunder-org/jaunder#1549`. A boundary change additionally requires a numberless
ADR draft and architecture projection. An excluded finding cites the exact
accepted boundary. After correction or disposition, rerun the proof that found
it before checking the task.

## Task outline

- [x] Task 1: Establish baselines and bootstrap the Theme repository
  - Contract: capture the pinned upstream reference and transient Studio
    **Before** images with the spec's deterministic routes, states, viewports,
    authentication, and color modes before presentation work; bootstrap `main`
    with only `README.md`, `LICENSE`, and `.gitignore`.
  - Verification: the repository is public with `main` as its default branch;
    its history shows no substantive Theme files in the bootstrap; every visual
    baseline has recorded reproduction conditions and an artifact path.

- [x] Task 2: Prove generated source and local package validation
  - Contract: a substantive branch owns pinned Tailwind source and lock data,
    generated root `style.css`, schema-1 `theme.json`, declared WOFF2/raster
    `assets/`, provenance, maintenance and installation docs, and a caller
    workflow. Generation is deterministic; selectors target only Style Contract
    1 hooks.
  - Verification: a clean checkout regenerates byte-identical `style.css` and
    fails on drift; package assets and defaults resolve; the pinned Jaunder CLI
    accepts the root with `jaunder theme check`; repository checks pass.

- [x] Task 3: Prove the canonical workflow artifact
  - Contract: the caller pins 40-hex Jaunder and reusable-workflow revisions and
    runs repository generation before Jaunder's canonical workflow.
    `preview.png` is committed; `theme-package.zip` is generated only.
  - Verification: the workflow reproduces `preview.png`, uploads a deterministic
    ZIP, and a clean rerun produces the same package bytes.

- [ ] Task 4: Prove the pre-release Studio lifecycle
  - Contract: the workflow-built ZIP is the sole input to import, private draft
    preview, publish, and explicit selection in a disposable Jaunder instance.
  - Verification: each lifecycle transition succeeds without bypasses; Studio
    remains unthemed and provides recovery; a signed-out browser receives the
    selected immutable revision.

- [ ] Task 5: Prove route, state, asset, and visual fidelity
  - Contract: the installed workflow artifact exercises the approved route and
    optional-state matrix, package font, logo/header default, and every header
    pool entry. **After** captures reproduce all **Before** conditions.
  - Verification: Local, author, tag, and permalink checks pass; transient pairs
    and pinned upstream references are linked from the Theme PR; human review
    approves the spec's named fidelity characteristics.

- [ ] Task 6: Prove accessibility and trusted owner controls
  - Contract: run the spec's WCAG scans and manual keyboard, focus, contrast,
    reflow, zoom, reduced-motion, overflow, and long-content checks; run
    ADR-0188's Chromium/Firefox and Safari/WebKit owner-actions matrix.
  - Verification: every numeric and scenario threshold in the spec has retained
    evidence, including multiple owned Posts, scrolling, narrow placement, menu
    use, and custom-theme isolation.

- [ ] Task 7: Finalize immutable pins and review surfaces
  - Contract: required Jaunder fixes land first. The Theme branch then pins the
    resulting commit, or the tested Jaunder `main` when no fix was needed. The
    Theme PR references `jaunder-org/jaunder#1549` and owns all substantive
    Theme files. Each repository PR retains its separate merge-approval halt.
  - Verification: every earlier proof passes against the final pin; the Theme PR
    and every required Jaunder PR are current and green; `PROVING-GROUND.md`
    accounts for every observed failure.

- [ ] Task 8: Release and repeat installation with the released artifact
  - Contract: after Theme PR merge, an initial SemVer tag invokes the pinned
    workflow; its released `theme-package.zip` is the sole installation input.
  - Verification: the release asset matches the reviewed workflow artifact;
    repeat import, preview, publish, selection, and signed-out route checks with
    that asset. Record the release, installation evidence, and blocker
    dispositions on #1549 before closing the issue.

## Cross-repository execution contract

- Jaunder source changes stay in this checkout and branch. Theme source work is
  performed in a dedicated checkout/session for `jaunder-org/theme-tailwind`;
  neither repository is nested in or used as a worktree of the other.
- A Theme PR may begin against the current pin, but cannot become the release
  candidate until all required Jaunder changes are merged and the immutable pin
  is updated.
- Planning artifacts and any Jaunder correction form the Jaunder review surface;
  Theme source, generated artifacts, and authoring documentation form the Theme
  review surface. Both point to the same issue and blocker log.
- The canonical ZIP is generated, never committed. `preview.png` and generated
  `style.css` are committed because ADR-0197 defines them as reviewable source
  artifacts.

## Risk checks

- Preserve ADR-0184 package isolation, scoping, URL rules, immutable
  publication, deterministic rendering, and trusted-control separation.
- Preserve ADR-0188's Chromium/Firefox plus Safari/WebKit owner-actions evidence
  and its multiple-Post, scrolling, narrow-layout, menu, and isolation matrix.
- Preserve ADR-0197's root-input boundary and canonical CLI/workflow ownership;
  repository preprocessing must run before, not replace, those authorities.
- Keep the upstream MIT notice and exact reference commit without claiming merge
  compatibility or copying assets whose licensing is not established.
- Do not let generated Tailwind utilities exceed package rule/nesting limits or
  target incidental markup; pruning must retain every spec-required route/state.
- Keep transient visual-proof pairs out of product snapshot corpora and
  recapture them after any later presentation-affecting change.
- Do not tag or release before the Theme PR merge, and do not close #1549 before
  the released artifact completes the real Studio installation proof.
