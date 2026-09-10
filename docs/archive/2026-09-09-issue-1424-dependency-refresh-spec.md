# Issue #1424 — Refresh build tools and dependencies

## Outcome

Jaunder builds, tests, and runs against a repository-wide snapshot of the newest
stable build tools, libraries, environments, and compatibility baselines
available when implementation starts. The refresh preserves Jaunder behavior and
repository invariants except for the explicitly advanced compatibility
baselines.

## Load-bearing decisions

- The refresh covers every tracked build or runtime version surface: the Rust
  toolchains and all three Cargo workspaces; npm and browser/e2e tooling; Nix
  inputs, packages, and source overrides; CI Actions and runner images; and
  supported environment baselines such as database major and TypeScript target.
- “Latest” is frozen once, immediately after this spec is approved and
  implementation begins. Releases published later do not reopen the snapshot
  unless one is needed to correct the upgrade.
- Select the newest stable release for each dependency or tool and perform the
  bounded source or configuration migration it requires. Existing prerelease,
  fork, revision, and exact compatibility pins are re-evaluated rather than
  carried forward mechanically.
- A deliberate workaround remains only when its documented invariant is not
  satisfied by a current stable upstream release. Coupled identities move as a
  unit: in particular, the Atom namespace fork stays synchronized across Cargo,
  the flake input, and Crane vendoring; Playwright's npm packages stay exactly
  aligned with the Nix runtime and browser set.
- Refresh every generated lock to its newest resolvable dependency graph,
  including transitive packages without direct manifest changes: the product,
  xtask, and tools Cargo locks, the e2e npm lock, and the flake lock.
- Advance compatibility baselines as well as package versions. This includes
  compiler/tool output targets, database major versions, CI runner operating
  systems, and test/deployment environment baselines where the repository
  carries them. It does not authorize application data-format or public protocol
  changes.
- Preserve the repository's existing source-of-truth boundaries and regenerate
  derived hashes from their owning manifests or locks. Do not introduce a second
  package manager, downloaded browser path, tool resolver, or hand-edited
  generated file.
- If the newest stable choice is blocked by an upstream incompatibility, lacks a
  stable replacement for a required prerelease, or demands unrelated
  architecture or behavior changes, stop for a case-by-case user decision. Do
  not silently retain the old pin, choose an older version, or widen scope.
- This one-time refresh establishes no recurring dependency-update policy and
  changes no domain term. Existing ADRs remain authoritative; any newly required
  architectural trade-off is recorded separately before implementation proceeds.

## Acceptance

- A version inventory taken at implementation start accounts for every tracked
  direct pin, source revision, toolchain, environment baseline, CI Action, and
  generated lock; each is updated to the selected current version or has an
  explicitly approved case-by-case disposition.
- Rust stable and the diagnostic nightly, their Fenix hashes and components,
  Cargo manifests, and all three Cargo lockfiles form buildable, internally
  consistent dependency sets.
- Flake inputs and explicit Nix source/package overrides are current and their
  generated revisions, hashes, follows relationships, and vendored sources are
  consistent; the deployable Jaunder package still builds.
- npm dependencies and the npm lock are current. Playwright test, runtime,
  browser binaries, and provisioned modules resolve to one exact version, npm
  browser downloads remain disabled, and the derived npm dependency hash is
  current.
- CI Actions, runner images, database/tooling majors, TypeScript target, and
  other repository-carried compatibility baselines match the approved snapshot
  and retain their documented functional roles.
- Dependency policy, license, source, advisory, formatting, type-check,
  compilation, host-test, wasm, doctest, coverage, and static-analysis checks
  pass without suppressions or exemptions added solely to make the refresh pass.
- Full `cargo xtask validate` passes, including every SQLite/PostgreSQL ×
  Chromium/Firefox end-to-end combination, demonstrating unchanged observable
  application behavior on the refreshed stack.

## Boundaries

- No product feature, domain behavior, public protocol, persistent application
  data format, or opportunistic refactor is changed merely because an upgrade
  exposes the opportunity.
- No dependency updater, bot policy, support-window policy, or new
  package/source selection abstraction is introduced.
- Compatibility corrections are the smallest changes that satisfy the selected
  versions. A correction that crosses an architectural or behavioral boundary
  returns to the user for a case-by-case scope decision.
