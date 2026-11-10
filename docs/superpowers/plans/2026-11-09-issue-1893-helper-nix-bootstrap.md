# #1893 helper Nix bootstrap implementation outline

> **Outline approved** by the separate direct human response “approved” to
> Parent's four-step outline brief, following cold review
> `result:a2-outline-review#2` (no findings). The approved scope contract is
> `docs/superpowers/specs/2026-11-09-issue-1893-helper-nix-bootstrap.md`. The A2
> v3 proposal snapshot is
> `/tmp/issue-1893-helper-bootstrap-intake/approved-proposal-v3.snapshot.md`
> (SHA-256 `a79f0939488c0b31e3ccce653f50ba597a3ff2109da4bbc9daeeeeb858175b01`);
> its direct human response was “yes, do it through nix”. Preserve the scope
> boundary. Before Task 1, commit these approved spec and outline versions using
> the ordinary hook-backed planning commit.

## Execution prerequisite

The outline is approved but not executable until these approved artifact
versions are preserved. Read `dev-cycle-iterate` and the matching Jaunder
iterate overlay, then make the ordinary hook-backed planning commit containing
**both** approved A2 spec and outline before Task 1. If that hook is blocked by
the shared SQLite advisory issue, retain its failure and coordinate with the
Manager-owned repair owner; do not edit/ignore dependencies, bypass the hook, or
repeat an unchanged failing gate. Do not implement before this preservation
step.

## Execution summary

Add one narrowly filtered, debug-profile `ux-sandbox` Nix package; change only
`just sandbox` to
`exec nix run --no-update-lock-file --no-write-lock-file .#ux-sandbox -- "$@"`;
and request an ordinary build of that package in the existing Cachix-configured
CI job. Test the package's source boundary before wiring the front door. Reuse
the current Crane/source-filter and Cachix action patterns; do not add a cache
framework, alter cache trust/filter settings, or claim upload, substitution, a
hit, or faster warm invocation without evidence.

New untracked helper inputs are the explicit limitation: the Git-flake source
snapshot does not automatically include or refuse them. Users must `git add` or
`git add -N` such files before Nix can build them. Tracked dirty and
intent-to-add inputs participate. Cargo currently reuses warm helper outputs;
this change promises neither a compile on every Cargo invocation nor faster warm
behavior under Nix.

## Task outline

- [x] **Task 1: Build a bounded runnable helper package.**
  - **Files:** add the helper-specific package definition in
    `nix/ux-sandbox.nix` following the isolated host-package pattern in
    `nix/xtask.nix`; expose it as `packages.ux-sandbox` from `nix/packages.nix`.
  - **Contract:** source is `ux-sandbox` plus its computed Cargo path-dependency
    closure, root workspace manifests/configuration, root `Cargo.lock`, and
    pinned toolchain. Reuse `cargoPackageClosure`, `cargoMemberSource`,
    `workspaceMembers`, and workspace placeholders for excluded members. Vendor
    dependencies for this bounded source and build only `ux-sandbox` with its
    current debug profile and one runnable `ux-sandbox` program. Do not inherit
    product `commonArgs`/`hostArgs`, application source/assets, CSR inputs,
    product `cargoArtifacts`, or unrelated workspace sources. Do not add a
    runtime admission API or pre-bootstrap refusal for untracked files.
  - **Verification:** realize `.#ux-sandbox` on the supported host and check the
    store output is runnable. Perform one finite derivation/source-boundary
    proof: tracked helper Rust changes its derivation; unrelated application
    Rust, CSR, public-asset, and documentation edits do not. Root/workspace
    manifests, lockfile, toolchain, or required helper dependencies may change
    it; do not claim other shared-lock changes are irrelevant. In the same
    bounded proof, use only task-owned temporary inputs with exact baseline
    preservation to observe that tracked-dirty and intent-to-add helper inputs
    participate in Git-flake capture, while a new untracked helper input is
    omitted without refusal. Do not touch another checkout/worktree, add source
    staging/refusal, or create a persistent framework or broad input matrix.

- [x] **Task 2: Switch only the public Just bootstrap.**
  - **Files:** change the `sandbox *args` recipe in `Justfile`; adjust
    `ux-sandbox/tests/public_command.rs::just_forwards_literal_argv_without_interpolation`.
  - **Contract:** retain `[ positional-arguments ]`, the active-checkout flake,
    and literal argument forwarding. Replace only the Cargo build plus target
    executable with
    `exec nix run --no-update-lock-file --no-write-lock-file .#ux-sandbox -- "$@"`.
    The recipe itself neither stages files nor detects omitted untracked inputs.
    Preserve the helper's application-artifact selection, profiles,
    source-admission policy, locks, lifecycle, and error behavior exactly.
  - **Verification:** adapt the existing isolated Just argv test to use a stub
    `nix` executable that records the exact `run`, lock flags, flake attribute,
    `--`, and caller arguments, including empty/space/metacharacter values.
    Confirm Just no longer invokes Cargo for the helper front door. Run the
    focused public empty-sandbox smoke against the real selected Nix executable
    only after the package is available; retain normal owned teardown.

- [ ] **Task 3: Request the package build through existing CI publication.**
  - **Files:** add one build step to the existing `validate-test-checks` job in
    `.github/workflows/ci.yml`, after `./.github/actions/setup-ci` configures
    Cachix; no changes to `.github/actions/setup-ci/action.yml` or cache policy.
  - **Contract:** invoke
    `nix build --no-update-lock-file --no-write-lock-file --no-link .#ux-sandbox`.
    The existing Cachix v17 `jaunder-org` action and
    `pushFilter: "jaunder-coverage|jaunder-e2e"` remain unchanged; this helper
    name does not match the exclusion filter and is eligible for ordinary
    publication. Build/configuration is not proof of upload or later
    substitution. Do not cache test verdicts or add tokens, upload steps,
    substitution logic, cache trust, or new publication infrastructure.
  - **Verification:** the current CI lane must actually request and complete the
    helper build. Claim publication/substitution only if the existing retained
    CI/cache evidence directly shows it.
  - **Local state:** the build request is wired and inspected. Actual CI
    execution/completion and any publication/substitution evidence remain
    pending.

- [ ] **Task 4: Conform, project enduring docs, and retire only A2 planning
      artifacts.**
  - **Files:** after implementation conformance review, update the
    helper-bootstrap descriptions in `CONTRIBUTING.md`, `docs/ARCHITECTURE.md`,
    and `docs/adr/drafts/standalone-ux-sandbox-lifecycle.md`. In the ordinary
    final cleanup commit, delete only the new A2 spec and outline:
    `docs/superpowers/specs/2026-11-09-issue-1893-helper-nix-bootstrap.md` and
    `docs/superpowers/plans/2026-11-09-issue-1893-helper-nix-bootstrap.md`.
  - **Contract:** record the Nix package/Just path, explicit untracked Git-flake
    limitation, and unchanged application-artifact/lifecycle boundary. Preserve
    the Cargo warm-reuse caveat and make no performance/cache-hit claim. Keep
    the numberless ADR draft proposed; do not promote it. Never restore or
    delete the already-retired A3 spec/outline as part of A2 cleanup.
  - **Verification:** after conformance, check the three enduring descriptions
    against the implementation, source filter, and CI request, then retire only
    the two completed A2 planning artifacts under ADR-0217's final-cleanup
    lifecycle. Ordinary post-implementation verification and commit hooks remain
    those of `dev-cycle-iterate`/`jaunder-iterate` and `dev-cycle-commit`; this
    outline does not replace or duplicate their gate checklist.
  - **Local state:** enduring descriptions are projected. Keep both A2 planning
    artifacts tracked while Task 3 CI completion remains pending; final cleanup
    and Task 4 completion follow conformance of that remaining obligation.

## Risk checks

- Do not change `ux-sandbox` application behavior or its source admission for
  application artifacts; do not change debug/release-CSR selection, helper
  lifecycle, seed subprocess, reset, locks, signals, terminal custody, or exact
  child status.
- Do not modify Cachix configuration/filter/trust, dependencies, Nix resources,
  `flake.lock`, app profile, #1923 staging permissions, Python, or any cache
  framework. No helper cache hit or warm-speed guarantee.
- If the shared SQLite advisory issue blocks an existing hook/gate, preserve its
  failure and evidence and coordinate with its Manager-owned repair owner; do
  not suppress it, alter dependencies, bypass hooks, or repeat an unchanged
  failure.
- Focused package/source-boundary, Just argv, and public-path proofs precede
  ordinary CI. Full project acceptance and merge remain separate; this outline
  grants neither.
