# Issue #2020: failed E2E lanes and their copied-out diagnostics

## Outcome

A failed E2E lane's producer reports truthfully what it recovered. It stops
printing a false "diagnostics unavailable" warning when it has in fact lifted
the VM's artifacts. It warns when it really has lifted none.

## What the investigation established

The issue assumed the artifacts are lost. They are not, locally or on CI:

- A failed `nixosTest` runs `mkdir -p $out; nixos-test-driver -o $out`. Under
  `--keep-failed`, Nix leaves that output at its real store path,
  world-readable. The kept build directory holds only `env-vars`. A minimal
  failing `nixosTest` that copied a file out of the VM confirmed this on local
  Nix 2.34.
- Locally, `dev e2e-producer sqlite-chromium-serial-special` was run with an
  injected failing Playwright test. The script's existing ADR-0037 branch lifted
  every artifact into `.xtask/diagnostics/e2e-<lane>/` (`nix eval` the check's
  `outPath`, then copy `*-<lane>.*`). That covered the Playwright report and the
  `test-results/` tarball, which held the failing test's `trace.zip`, screenshot
  and `error-context.md`. It also covered the capture, both journals, and the
  phase and lane manifests.
- On CI, the run the issue cites was run 44773522256: lane
  `sqlite-chromium-unsplit`, Nix 2.35.6 from `cachix/install-nix-action`. It was
  an inner Playwright timeout (`exit 124`). Its uploaded artifact holds the 77
  MB capture, the `test-results/` tarball, both journals, and the phase and lane
  manifests. The Playwright JSON report and the duration manifest are absent:
  Playwright writes them only when it finishes, and the timeout killed it. No
  carrier can recover files that were never written. #2011's per-test lines in
  `build.log` cover that case.
- Both runs still printed
  `warning: current failed-build diagnostics unavailable or ambiguous`. That
  comes from a second branch that parses `note: keeping build directory` and
  looks for `emit-out/diagnostics/`. Only the coverage and doctest derivations
  write that layout, and this script never builds them. So the warning fires on
  every failed lane, and it misled the issue report.
- Two documents describe the wrong mechanism. The `nix/checks.nix` comment above
  the capture helpers says failed-run artifacts "live in the --keep-failed build
  dir for the host's rescue_diagnostics", a function that no longer exists.
  `docs/ARCHITECTURE.md` (the "e2e" section, ~line 4482) says the script
  recovers from the build directory. ADR-0037 §3 correctly says the kept output
  path.

## Approach

- `scripts/e2e-workflow`: delete the kept-build-directory branch and its
  warning. On a failed build, count the lane artifacts the `outPath` lift
  copied. If there are none, print one warning naming the check, whatever the
  reason: evaluation failed, the path is missing, or the directory has no lane
  files (for example a boot failure before the copies). The build's own exit
  status stays as it is.
- `tools/dev/tests/test_e2e_workflow.py`: delete the two kept-build-directory
  tests, the `kept()` helper and `FAKE_NIX`'s `kept-note` replay. Add one test,
  parametrized over three cases: evaluation fails, path missing, empty output.
  Each case asserts the new warning, the build's exit status, and that the old
  "unavailable or ambiguous" text is absent. The existing
  `test_a_failed_producer_lifts_available_evidence_but_never_verifies_it` covers
  the successful lift from a failed output.
- `nix/checks.nix` comment and `docs/ARCHITECTURE.md` sentence: name the kept
  output path and `scripts/e2e-workflow`. ADR-0037 stays as it is: its decision
  is what the code does. Nix comments change no derivation.

Alternative considered: point the kept-directory branch at wherever the driver
writes. That is redundant, because the output path already carries the same
files, and ADR-0037 rejected the build directory as unreadable and configurable.

## Acceptance

- On the host (it needs a real Nix store),
  `pytest tools/dev/tests/test_e2e_workflow.py` passes, including the three new
  no-lift cases.
- `scripts/e2e-workflow` no longer contains `keeping build directory` or
  `emit-out`.
- The local and CI evidence above already proves the failed-lane lift through
  `outPath`. No new VM run is needed, because the deleted branch never
  contributed any files.

## Boundaries

- No change to the VM test script, artifact names, CI upload, or the
  coverage/doctest `emit-out` derivations. No new recovery carrier.
- Existing local exposure, unchanged: if a run fails before Nix restarts the
  lane's derivation (for example when a dependency fails), the `outPath` lift
  can pick up an earlier run's kept failed output. Nix clears an invalid output
  only when it starts that derivation. CI runners are fresh, so this affects
  only local runs. The deleted branch did not guard against it either.
