# Issue #2008 — lean shells for the Nix-only CI lanes

## Outcome

The `Validation hermetic`, `Validation test checks` and `Validation coverage`
jobs stop entering the full `.#ci` devShell. Each one, including its trace
summary step, enters a lean shell that holds only what those lanes run on the
host.

Baseline is run 44383337174 (2026-11-18), measured from the start of the `dev`
step to the start of the `dev` root span:

- `.#ci` took 69 s (hermetic), 74 s (test-checks) and 68 s (coverage).
- In the same run, the existing lean `ci-e2e` took 39–55 s to enter (40 s in
  `E2E catalog`). Part of that is flake fetch and evaluation, which the lanes'
  later `nix build`s would otherwise pay anyway.

So the expected saving is about 25–35 s per lane. The before and after numbers
are recorded on #2008; this spec does not promise a figure.

## What these lanes run on the host

| Lane        | Host steps                                                                                                         |
| ----------- | ------------------------------------------------------------------------------------------------------------------ |
| hermetic    | `git status`, nine `nix build .#checks…`, `nix build .#csrBundle`, `wasm-audit`                                    |
| test-checks | `git status`, `nix build` of wasm-tests, doctests (+gate), elisp-coverage-producer, `coverage-check elisp-consume` |
| coverage    | `git status`, `nix build` of coverage (+gate), `coverage-check policy`                                             |

Every step also uses `dev` (Python) and `nix`. The only subprocess that
`coverage-check policy` and `elisp-consume` start is `nix`. `dev trace summary`
is pure Python. The platform-contract `nix eval` runs only in the host lane.

None of these lanes needs the Rust toolchain, a C compiler, Emacs, PostgreSQL,
Node, Playwright, its browsers, chromium, the OTel collector or the formatters.
Those run inside Nix derivations, which carry their own closures.

**Dependency: #1968.** Today `wasm-audit` and `coverage-check` are compiled on
the host with `cargo build`, taking 29–62 s per lane, so these lanes still need
the toolchain. #1968 makes every `dev` tool a cached Nix output
(`nix build .#<pkg>`, as `e2e-evidence` already is) and deletes the cargo build
path; it has been agreed with that session and approved by the user. #1968 also
owns setup-ci's host-tools cache step. This issue lands on top of #1968 and does
not touch `nix/packages.nix`, `tools/dev/` or setup-ci.

## Approach and tradeoffs

- **One lean shell, shared with e2e.** `ci-e2e` is already this kind of shell:
  - bash, coreutils, findutils, nix, git, `dev`, traced-run and jq;
  - the hook that binds TMPDIR to the checkout;
  - the Linux `LD_LIBRARY_PATH` export of Nix OpenSSL and dav1d.

  Rename it to `ci-nix`, since it now serves every job whose verdicts are Nix
  builds. Point the three validation lanes at it, for both their `dev` step and
  their trace summary step. The e2e catalog, lane and reconcile jobs use the
  same shell under the new name.

- **Make it smaller.** Remove `cargo`, which is left over from the xtask era: no
  host step in the e2e path or in these lanes runs it. Build the shell with
  `mkShellNoCC`, so `nix develop` no longer substitutes stdenv's gcc, binutils
  and glibc-dev; nothing here compiles on the host.

  Keep the `LD_LIBRARY_PATH` export. CONTRIBUTING promises that every project
  shell sets it, and it costs only two small libraries.

- **Stays Linux-only**, as `ci-e2e` is.

- **Unchanged:**
  - `validate-host` and `performance.yml` stay on `.#ci`. The host lane really
    does use most of `ciInputs`, and its run takes 15 minutes.
  - `default` and `mutants` are untouched; `default` stays complete for local
    work.

- **Alternatives rejected:**
  - One shell per lane: three near-identical definitions of one need.
  - Keeping the `ci-e2e` name: it would be wrong for the validation lanes.
  - No devShell at all (the runner's nix plus a Nix-built `dev`): the shell's
    own cost after this change is mostly a few small store paths. The evaluation
    part of the ~40 s floor would move to the first `nix build` rather than
    disappear. Dropping the shell would lose the TMPDIR and pinned-git contract
    for a few seconds at most.

- **Accepted costs:**
  - These lanes can no longer run host cargo. A future host step that needs it
    must use a Nix-built tool or run in the host lane.
  - `pr-observer` checks the exact catalog command (`workflow.rs:553`) against
    the workflow at the observed run's head. An observer built from a branch
    that has not rebased past this change will reject the new command until it
    rebases. This lasts for one rebase and is not worth a compatibility path.

## Pinned binaries

Every command the three lanes run inside `nix develop .#ci-nix` resolves to a
store path from the shell (`nix`, `git`, `dev`, `jq`, `traced-run`) or is a Nix
build output, with no fallback to the runner's PATH.

Steps that already ran outside a devShell are out of scope and remain as they
are: test-checks' `.#ux-sandbox` and Emacs cachix builds, and the storage
probes.

## Acceptance

Proof to execute:

- **CI run.** A CI run on the PR passes all four validation lanes and the e2e
  jobs. The hermetic, test-checks and coverage logs show `nix develop .#ci-nix`
  for both the `dev` step and the trace summary step.
- **Measurement.** Post on #2008 the shell entry time for those three lanes and
  for `E2E catalog`, measured as in the baseline, next to the 69/74/68 s and 40
  s figures.
- **Shell tests.**
  `cargo test --locked --manifest-path tools/Cargo.toml -p host-proof --test dev_shell_tmpdir --test workflow_callers`
  passes.
- **Observer tests.**
  `cargo test --locked --manifest-path tools/Cargo.toml -p pr-observer` passes.
- **Platform contract.**
  `nix eval --impure --expr 'import ./nix/platform-contract.nix { flake = builtins.getFlake ("git+file://" + toString ./.); }'`
  returns `true`.

Deliverables to commit:

- `nix/dev-shells.nix`: `ci-e2e` becomes `ci-nix`, built with `mkShellNoCC` and
  without `cargo`. Its comment names the jobs that use it.
- `.github/workflows/ci.yml`: the three lanes and the e2e jobs enter `.#ci-nix`,
  and the step name "E2E disk observation (after ci-e2e shell entry)" follows
  the rename.
- `workflow_callers.rs` expects `.#ci-nix` for those three lanes and `.#ci` for
  host.
- `dev_shell_tmpdir.rs`:
  - covers `ci-nix` in place of `ci-e2e`;
  - asserts that the pinned `nix`, `git`, `jq`, `traced-run`, `dev`, bash,
    coreutils and findutils are present;
  - asserts that cargo and the Rust toolchain are absent.
- `pr-observer`'s expected catalog command uses `.#ci-nix`.
- `nix/platform-contract.nix`: `ci-nix` exists exactly on Linux, and its
  `drvPath` is forced along with `default` and `ci`.
- `CONTRIBUTING.md`: the shell mentions near lines 68, 84, 92 and 844 name
  `ci-nix` and the jobs that enter it.

## Boundaries

- No changes to `nix/packages.nix`, `tools/dev/` or setup-ci; #1968 owns those.
- `ciInputs`, the host lane, the performance workflow and the mutants workflow
  are untouched.
- No new trace instrumentation; #2005 covers shell-entry visibility.
