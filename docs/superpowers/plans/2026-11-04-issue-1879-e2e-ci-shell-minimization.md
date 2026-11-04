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

- [x] Add the narrow shell preserving Cargo/Rust launcher and Nix-built xtask
      path, native runtime and exact `LD_LIBRARY_PATH`, fail-closed TMPDIR hook,
      `LC_ALL`, `TZ`, and `TZDIR`. Omit only proven host-only validation tools,
      theme-thumbnail Chromium, IDE provisioning and specified unused host
      environment variables. Keep uncertain inputs. Preserve all guest graph
      dependencies and broad `.#ci` behavior.
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
provisions IDE Node modules. Both require the launcher, Nix-built xtask,
Rust/Cargo dispatch and native libraries. VM tests retain
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

Before evaluation, record the bounded candidate/input path set available from
the existing CI setup/cache contract and already available derivation metadata
for the broad/narrow shell inputs, xtask, and exact lane. Capture each
candidate's starting validity (present or absent) and existing `narSize` without
fetching missing paths. Capture complete Nix evaluation logs from before each
shell/check evaluation so import-from-derivation (IFD), substitution, or local
build acquisitions triggered during evaluation are visible as evaluation-phase
activity, not mislabelled as initial state. Any path first discovered during
evaluation without a defensible starting-validity observation remains
unknown/unproven; do not scan the store to fill the gap.

After the normal producer has realized its shell, xtask, and exact lane outputs,
record their exact output identities and query their output closures/requisites
by store path. Refine the pre-execution candidate sets against these actual
closures by exact identity. Retain starting-validity evidence for every
candidate that can be compared. Subtract paths shared with xtask and lane output
closures before calculating shell-exclusive acquisitions. Count shell-entry and
evaluation acquisitions as well as producer-time acquisitions. Classify as newly
acquired only paths initially absent with logs establishing substitute copy or
local build; paths present initially are reuse. Sum `narSize` once per unique
acquired identity and report substitution and locally-built NAR bytes
separately; these are serialized NAR bytes, not filesystem-allocated bytes.
Exact closures or acquisition status lacking starting evidence remain unproven,
and cannot support a material-reduction claim.

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
correct in scope and retry only once prior work settles. Stop before
commit/push/PR/full gates for Lead review after focused implementation proof.
