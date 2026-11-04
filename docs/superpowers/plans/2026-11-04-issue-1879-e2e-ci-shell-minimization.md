# E2E CI host-shell closure minimization — implementation outline

> Governing approved design:
> `docs/superpowers/specs/2026-11-04-e2e-ci-shell-minimization.md`. Design
> approval: request `bdfeac26-ae92-4df4-85d0-e6ec03728eee` /
> `ci-e2e-shell-spec-v1`, exact “Approve narrow-shell design.” Outline approval:
> request `0e9e88eb-7b9b-454d-801b-14e26230d148` /
> `issue1879-ci-e2e-outline-v1`, exact “Approve outline and begin.” Both are
> recorded approvals; stale pending text is superseded. Implementation is
> authorized; merge remains separately gated.

## Scope

Add Linux `.#ci-e2e` only for `e2e-chromium` and `e2e-firefox-lane`, preserving
exact xtask commands, arguments and identities. Four validation producers and
`e2e-firefox-reconcile` stay on unchanged `.#ci`; aggregate gates/artifacts and
all other jobs remain unchanged. Preserve all backend/browser behavior, lane
catalog, VM/KVM, diagnostics, evidence, Firefox census/reconciliation and
coverage authority.

## Tasks

- [x] Add the narrow shell preserving the Cargo external-subcommand launcher and
      Nix-built xtask path, native runtime and exact `LD_LIBRARY_PATH`,
      fail-closed TMPDIR hook, `LC_ALL`, `TZ`, and `TZDIR`. Omit only proven
      host-only validation tools, theme-thumbnail Chromium, IDE provisioning and
      specified unused host environment variables. Keep uncertain inputs.
      Preserve all guest graph dependencies and broad `.#ci` behavior.
- [x] Substitute the shell selector in only the two named producer jobs. Verify
      all others unchanged.
- [x] Add named, bounded same-runner `df` observations after setup, after shell
      entry, after producer, and final after artifact upload. Each probe emits
      `observation_status=success` or `observation_status=failure exit=N` and
      exits successfully, so diagnostics cannot suppress or replace producer
      verdicts. Full CI verdicts remain required and have not yet been run.

## Host call-path justification

`e2e` dispatches Chromium to `e2e_combo`, which builds the Nix check,
validates/lifts diagnostics and duration/boot evidence, performs flaky
scan/phase checks and SQLite/Chromium server-function coverage verification.
`e2e-lane` validates catalog identity, builds one lane check, recovers
diagnostics and runs the same Rust validators/flaky/phase checks. Neither path
uses validation/coverage/format/wasm executables or theme-thumbnail tooling nor
provisions IDE Node modules. Both require the launcher, Nix-built xtask, Nixpkgs
Cargo as the external subcommand dispatcher and native libraries. The traced
launcher path (`scripts/cargo-xtask`) only acquires `.#xtask` and executes its
compiled host binary; the Nix derivation owns the pinned build toolchain. The
host does not compile Rust itself. The local shell-entry test passed and its log
still showed Nixpkgs Cargo references to rustc and rustc-wrapper; this change
removes the Fenix aggregate only as a direct shell input and does not claim a
closure or byte reduction until paired measurements. VM tests retain
browser/font/backend/Node/telemetry/KVM/nixos-test dependencies. `TZDIR` stays
pending stronger evidence.

## Paired acquisition proof protocol

This protocol compares a bounded pre-execution candidate/input inventory with
exact output closures only after authorized producer runs normally create them.
It does not realize roots solely for sizing. Use comparable fresh `ubuntu-24.04`
runners for the same backend/browser/lane identity, checkout, setup action,
credentials, Nix configuration and cache policy: one arm runs the unchanged
producer through `.#ci`, the other through `.#ci-e2e`. Never run both arms in
one Nix store. Preserve and compare starting cache/store-validity conditions; if
they are not comparable, report the acquisition comparison inconclusive.

For each paired runner, begin retaining complete Nix logs before its first flake
evaluation, including IFD. Before evaluation, record present/absent validity and
existing `narSize` only for bounded candidate identities already available from
setup/cache metadata; never fetch missing paths. Then evaluate the exact shell
and producer `.drvPath` values. Evaluation itself may substitute/build IFD
inputs, so preserve those evaluation-phase Nix logs separately. Recursively
expose resulting derivation metadata before normal shell/producer realization:

```bash
shell_drv="$(nix eval --raw ".#devShells.x86_64-linux.$shell.drvPath")"
xtask_drv="$(nix eval --raw .#packages.x86_64-linux.xtask.drvPath)"
producer_drv="$(nix eval --raw ".#checks.x86_64-linux.$producer_check.drvPath")"
nix derivation show --recursive "$shell_drv" "$xtask_drv" "$producer_drv" > "$RUNNER_TEMP/e2e-drvs.json"
# Record candidate identities; the check name comes from the live backend/catalog matrix.
jq -r '.derivations[] | ([.inputs.srcs[]?] + [.outputs[]?.path?])[] | "/nix/store/" + .' \
  "$RUNNER_TEMP/e2e-drvs.json" | sort -u > "$RUNNER_TEMP/e2e-candidates.txt"
mapfile -t candidates < "$RUNNER_TEMP/e2e-candidates.txt"
# This query checks validity only; it does not substitute invalid paths.
nix-store --check-validity --print-invalid "${candidates[@]}" \
  > "$RUNNER_TEMP/e2e-invalid-post-evaluation.txt"
```

The checked-in bounded seed `.github/workflows/issue1879-acquisition-seed.txt`
contains 994 broad-shell closure identities that are absent from narrow-shell,
xtask, and exact lane input closures, plus the 14 previously observed candidates
(1,008 unique paths, 67,339 bytes). It was derived from current local recursive
derivation metadata: `.#devShells.x86_64-linux.ci`
`/nix/store/y13ria0fc018cvkdww90n3z349l2vsrh-nix-shell.drv`,
`.#devShells.x86_64-linux.ci-e2e`
`/nix/store/kqpyg530yzypaqy3nwsvi5mhifwrgvqx-nix-shell.drv`,
`.#packages.x86_64-linux.xtask`
`/nix/store/3086g4v51mmmxjprmdly3a1jdfcz3isw-jaunder-xtask-0.1.0.drv`, and
`.#checks.x86_64-linux.e2e-sqlite-firefox-ordinary-2-of-2`
`/nix/store/y3vcl0d3flj4dm3l8drk8jfjv2xk29vi-vm-test-run-jaunder-e2e-sqlite-firefox-ordinary-2-of-2.drv`.
The extraction follows recursive `inputs.drvs`, collecting each derivation's
`inputs.srcs` and output paths, then subtracts narrow, xtask, and lane
candidates from the broad-shell set; both shell-output sentinel paths are
excluded. This provides a useful bounded broad-exclusive starting inventory
without building any missing path. The exact roots and set counts are
re-evaluated against the final diff before freezing the packet; changes in those
roots require regenerating the seed.

Before evaluation, snapshot all seed candidates and `narSize` only for paths
proved valid. Query failures stay unknown. Post-evaluation derivation candidates
not in the seed stay unknown at start; evaluation logs can prove a completed
copy/build only when the exact identity is subsequently valid. Never scan
`/nix/store` or realize missing paths for measurement.

After `nix print-dev-env --json` (which can realize dependencies), extract store
roots from the actual shell environment and query validity. For successfully
validated existing roots, save `nix path-info --recursive --json` metadata as a
separate shell-environment closure. This captures actual environment references
without requiring or building the mkShell output sentinel. Separately, after
normal producer execution, query exact xtask and producer output closures only
when those outputs already exist. Keep all three inventories distinct; refine by
exact path identity and subtract paths shared with xtask/producer before
calculating shell-exclusive acquisitions. For each phase, compare successful
validity snapshots: runner start, post-evaluation, after `print-dev-env`, after
shell entry, and after producer. Seed paths absent at runner start and present
post-evaluation are recorded as completed evaluation-phase acquisitions. A path
absent in a successful post-evaluation snapshot and valid at a later checkpoint
is an acquisition in that later interval. Never relabel a
post-evaluation-present path as initial reuse. Unseeded paths already valid
post-evaluation are explicitly retained as unknown-start evaluation candidates,
not acquisitions. Pair transitions with phase Nix logs to distinguish
substitution/local builds; a validity transition without attributable logs has
phase provenance unknown.

For exact shell-exclusive closure identities with NAR metadata, report a lower
bound summing unique identities with successful absent-to-valid evidence, and an
upper bound that additionally includes completed closure identities whose
starting state is unknown. Known-start-present identities are excluded. If an
output closure or NAR metadata is incomplete, report the numeric lower bound and
state that no finite upper bound is established for the missing identities; do
not substitute `df` deltas. Sum each unique `narSize` once. NAR bytes are
serialized path sizes, not filesystem allocation; unknowns preclude a point
estimate or an unqualified reduction claim.

Manager authorized exactly one temporary paired run for this recovery. The
existing CI workflow implements it as the `e2e-acquisition-pair` matrix with
`ci` and `ci-e2e` arms on independent fresh `ubuntu-24.04` runners, gated only
on PR #1882 and fixed to the same PR revision and producer identity
`sqlite-firefox-ordinary-2-of-2`. Both invoke the unchanged
`cargo xtask e2e-lane sqlite-firefox-ordinary-2-of-2` producer command. These
jobs have `contents:read`, no repository secrets, the same Cargo-cache key and
public read-only Nix cache, distinct diagnostic artifacts, and no dependency
edge into `e2e gate`; all official producer selectors, aggregates and artifact
names are untouched. Remove the temporary pair after this one run before the
final PR review. If either arm fails or evidence is incomplete, mark the pair
inconclusive and do not repeat it.

Each arm records runner image/cache-hit metadata and bounded `df` phases. Before
Nix evaluation it checks the previously observed Rust/Cargo candidate
identities; then it logs exact shell/xtask/check drv evaluation (including IFD),
captures `nix print-dev-env --json`, then records its actual environment-root
closures separately from xtask and producer outputs; it never requires a mkShell
output sentinel. Only already-valid roots are queried recursively.
Post-evaluation derivation candidates do not acquire retroactive starting
validity. The seeded broad-exclusive identities provide meaningful starting
coverage; other identities retain unknown start state. Phase checkpoints record
absent-to-valid transitions after evaluation. Nix logs attribute those
transitions to substitution/local build where possible; evaluation copies
without starting evidence remain a separate unknown-start category. Bounds are
computed from exact closure membership and unique NAR metadata; failed
validity/closure queries leave identities unknown, and incomplete sizes have no
finite numeric upper bound. `df` remains separate allocation evidence. No absent
shell sentinel/output root is built solely for sizing.

At each arm retain named `df -P / /nix/store "$GITHUB_WORKSPACE"` observations
at post-setup, after shell entry, after producer, and final-after-upload. `df`
deltas report filesystem allocation only, not per-path attribution. Do not
realize missing roots for sizing, scan the store, sum overlapping closure sizes,
or infer uniform runner capacity or causality from past runs. The historical 68G
baseline remains unexplained.

Full CI remains required after reviewed-tree approval: both SQLite/PostgreSQL
Chromium producers; all six Firefox backend/catalog lane identities; both
Firefox reconciliation jobs and exact census union; SQLite/Chromium coverage
verification; then `e2e gate` and `Validate (no e2e)`. Preserve logs, artifacts
and failures; missing closure identities are unavailable, not permission to
realize them solely for sizing.

## Risk, rollback, acceptance

No ENOSPC cure/capacity guarantee; observed root baseline remains unexplained.
No PR #1877 changes, cache purge/GC, resource flags, runner/image change, broad
scans, repeated broad shell path-info or closure-size summing. Rollback only the
two jobs to their former `.#ci` commands. Preserve any failures; diagnose and
correct in scope and retry only once prior work settles. This recovery delta
stops at focused proof and independent review before commit, push or CI. The
separately authorized temporary acquisition pair must run once after review;
remove both measurement-only jobs before the final review-ready delivery. If the
pair is inconclusive, do not rerun without renewed coordination.
