# Issue #1549: Tailwind Theme Package proving ground

## Outcome

Jaunder publishes a maintained external Theme Package at
`https://github.com/jaunder-org/theme-tailwind` that ports the recognizable
visual design of `tomowang/hugo-theme-tailwind` to Jaunder's public Style
Contract.

The repository is also the reference proof that an external author can build,
validate, release, install, publish, and maintain a nontrivial Theme Package
without working inside the Jaunder source tree. Concrete Jaunder defects or
missing capabilities exposed by that proof are fixed before the first release.

## Load-bearing decisions

- `jaunder-org/theme-tailwind` is a public, independent Jaunder theme
  repository, not a GitHub fork or a compatibility child of the Hugo theme.
- The port preserves the upstream MIT notice and records upstream commit
  `d6841f6c9d53155a3245d6472555860f7acb1cd0` as its initial visual and source
  reference. It makes no promise that later upstream commits can be merged.
- Later upstream ideas may be adopted selectively, but Jaunder's Style Contract
  and this repository's design become authoritative after the initial port.
- The theme targets Theme Package schema 1 and Style Contract 1. It uses stable
  semantic hooks rather than Hugo templates, Jaunder wrapper depth, incidental
  classes, or sibling positions.
- The port covers the complete supported public surface: Local, author
  timelines, tag routes, and Post permalinks. Timeline and tag evidence includes
  navigation, masthead, site title, main, Post list, multiple Posts, and a
  continuation; permalink evidence includes exactly one Post and no Post list.
  Across those routes, fixtures exercise present and absent title, summary,
  avatar, author handle, tags, attribution, and continuation states.
- The package includes supported package-local font and raster-image assets so
  the reference repository proves URL rewriting and font, logo, header-default,
  and explicit header-pool presentation rather than only the zero-asset path.
- Visual fidelity means preserving the upstream theme's recognizable card
  composition, typography, spacing, color, responsive behavior, and light/dark
  presentation where the Style Contract supplies the corresponding concepts. It
  does not mean reproducing Hugo's content model or feature set. The pinned
  upstream README screenshot and rendered home/list and single-Post views are
  the reference, judged alongside a before/after pair of the same deterministic
  Jaunder routes under Studio and the completed custom Theme.
- Dark presentation follows `prefers-color-scheme`. The package does not gain
  JavaScript or a theme-specific mode switch.
- Hugo-specific search, comments, analytics, PWA behavior, shortcodes, JSON-LD,
  multilingual machinery, and image processing are not Theme Package features
  and are excluded.
- SVG icons and other unsupported package members are not admitted merely for
  upstream fidelity. Copied assets require compatible licensing and must satisfy
  Jaunder's existing package allowlist; otherwise the port omits or replaces
  them with CSS or supported assets.
- The repository deliberately exercises ADR-0197's preprocessor contract. It
  owns a pinned Tailwind toolchain and any authoring source, commits generated
  root `style.css` as portable source of truth, and fails automation when a
  clean regeneration differs from that file.
- Root `theme.json`, `style.css`, and declared `assets/` remain the only Theme
  Package inputs. Tooling, source, documentation, provenance, workflow files,
  and `preview.png` remain repository support files.
- The repository calls Jaunder's canonical reusable theme workflow at immutable
  Jaunder and workflow revisions. Repository-owned generation and drift checks
  run before the reusable workflow.
- Canonical validation, thumbnail generation, deterministic packaging, and
  release attachment remain owned by Jaunder's CLI and reusable workflow rather
  than being reimplemented in the theme repository.
- Installation uses the existing explicit lifecycle: download the release ZIP,
  import it into `/themes`, preview it, publish it, and select the published
  Theme. The first release must prove that path with its actual workflow-built
  artifact.
- The repository is a proving ground for authoring and installation, so a
  concrete blocker in the Jaunder CLI, reusable workflow, Studio lifecycle,
  package compiler, or Style Contract is in scope for correction.
- A Jaunder change is justified only by evidence from the real package. General
  galleries, registries, remote package fetching, speculative extensibility, and
  unrelated upstream features remain separate work.
- Existing Theme Package isolation remains intact: no templates, JavaScript,
  server hooks, external resources, request randomness, or access to trusted
  owner controls.
- If a demonstrated blocker requires changing an accepted architectural boundary
  rather than correcting its implementation, the contradiction is surfaced and
  resolved through a new ADR before implementation continues.
- Any necessary Jaunder fixes land before the external repository pins the
  immutable Jaunder revision that contains them. Without such fixes, it pins the
  tested `main` revision.
- The new repository's bootstrap `main` contains only `README.md`, `LICENSE`,
  and `.gitignore`. Theme source, generated CSS, package members, preview,
  tooling, and workflow configuration are substantive work reviewed through a
  branch and pull request.
- An initial semantic-version tag produces a published GitHub release whose
  `theme-package.zip` is the exact artifact created by the pinned reusable
  workflow.
- Issue #1549 remains open until that released artifact has completed the real
  installation proof.

## Acceptance

- `jaunder-org/theme-tailwind` exists publicly with an MIT license, upstream
  attribution and reference commit, maintenance instructions, installation
  instructions, and the standard Theme Package repository layout.
- A clean checkout can regenerate `style.css` with the pinned Tailwind
  toolchain, and the repository's drift check proves the committed file is
  current.
- `jaunder theme check` accepts the repository through the canonical Jaunder
  environment without bypasses, copied validators, or weakened package limits.
- The pinned reusable workflow reproduces the committed 1200×800 `preview.png`
  and uploads a deterministic `theme-package.zip`.
- A tagged release publishes that exact ZIP as its release asset.
- The release ZIP imports as a private draft in `/themes`; the draft preview
  renders successfully; publishing succeeds; and explicit selection applies it
  to signed-out public pages.
- The installed release exercises the route and optional-state matrix defined
  above, plus a package logo, package header default, every entry in an explicit
  header pool, and the package-local font.
- Transient visual-proof artifacts pair Studio **Before** and completed-theme
  **After** captures for Local and a Post permalink at 1440×900 and 390×844, in
  light and system-dark modes, using identical deterministic content and
  authentication state. The handoff also places the pinned upstream screenshot
  and reference views beside those pairs and records human approval of the card
  composition, typography, spacing, color, responsiveness, and mode treatment.
- Automated accessibility scans report zero detectable WCAG 2.2 Level A or AA
  violations on Local, author, tag, and permalink routes. Manual evidence
  records complete keyboard reachability, visible focus, text contrast of at
  least 4.5:1 (3:1 for large text), UI/focus contrast of at least 3:1, reflow
  without two-dimensional scrolling at 320 CSS pixels, readable content at 200%
  browser zoom, and no nonessential motion when reduced motion is requested. The
  fixture includes headings, paragraphs, links, lists, block quotes, code, and
  an intentionally long unbroken token.
- Owner Post Actions evidence covers Chromium and Firefox plus a WebKit smoke or
  recorded Safari substitute at ADR-0188's browser floor. It includes multiple
  owned Posts, scrolling, narrow placement, opening and using the menu, and
  proof that custom-theme CSS cannot style the trusted controls. Studio remains
  unthemed and usable for recovery.
- Issue #1549 or its linked implementation PR retains a complete blocker log.
  Each observed CLI, workflow, Studio, compiler, or Style Contract failure
  records reproduction evidence, classification, and either the fixing Jaunder
  PR with regression proof or the concrete contract rule that makes it out of
  scope.
- Required Jaunder changes satisfy backend parity, relevant focused tests, the
  repository verification ladder, and their architectural documentation.
- The external repository PR and any Jaunder PR reference
  `jaunder-org/jaunder#1549`; the issue closes only after release installation
  evidence exists.

## Boundaries

- This work does not create a Theme registry, gallery, marketplace, or
  URL-based/one-click installer.
- It does not add executable content, external network resources, SVG admission,
  custom markup, or a second rendering system to Theme Packages.
- It does not promise pixel identity with Hugo pages whose structure or behavior
  has no Style Contract equivalent.
- It does not import Hugo configuration, front matter, taxonomies, templates,
  shortcodes, demo content, or application behavior into Jaunder.
- It does not make ongoing upstream synchronization a maintenance requirement.
- It does not weaken package validation, CSS scoping, immutable publication,
  trusted-control isolation, cache determinism, or accessibility to make the
  port easier.
- It does not treat the canonical thumbnail as sufficient route, responsive,
  browser, accessibility, or installation proof.
