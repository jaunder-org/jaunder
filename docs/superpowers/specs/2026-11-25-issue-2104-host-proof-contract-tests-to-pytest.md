# Issue #2104: host-proof's repository contract tests move to pytest

## Outcome

The five `tools/host-proof/tests` files that test no host-proof executable
(`dev_shell_tmpdir.rs`, `e2e_vm_contract.rs`, `workflow_callers.rs`,
`e2e_topology.rs`, `performance_workflow.rs`, about 1.9k lines) become pytest
modules under `tools/dev/tests` and are deleted. Every assertion survives, run
against the same real artifact, so the host-proof crate keeps only tests of its
own executables and its retirement (#2105) is no longer blocked by them.

## Approach and tradeoffs

Each Rust file becomes one pytest module that reads or executes the same real
thing it does today:

| Rust file                                           | pytest module              | Real execution kept                                                                                                                                                                                           |
| --------------------------------------------------- | -------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `dev_shell_tmpdir.rs`                               | `test_dev_shells.py`       | `nix eval` of each devShell's `shellHook` and its `ci-nix` contract, run in child Bash with real Git and Node                                                                                                 |
| `e2e_vm_contract.rs`                                | `test_e2e_vm_script.py`    | `nix-instantiate` of the real `nix/checks.nix` with primitive stubs; the generated test script's helpers executed; `nix/test_e2e_capture.py` run as a child `python -B -m unittest discover -s nix`, as today |
| `e2e_topology.rs`                                   | `test_e2e_matrix.py`       | `jq -f scripts/e2e-matrix.jq` over the real and mutated lane catalog; `ci.yml`'s matrix wiring (projection step, `fromJSON` matrices, reconcile `always()`)                                                   |
| `workflow_callers.rs` and `performance_workflow.rs` | `test_workflow_callers.py` | `.githooks/*` run with a fake `dev`; `ci.yml`/`performance.yml` parsed; aggregate verdict and trace-summary `run:` blocks executed under Bash                                                                 |

`performance_workflow.rs` joins `test_workflow_callers.py` because that module
already checks the performance job's trace upload, so both performance-job
contracts sit together. The matrix module keeps its own `ci.yml` assertions:
they describe the projection, not the jobs' verdicts.

What gets simpler in Python rather than merely translated:

- The VM-script harnesses are Python strings that Rust passes to a `python3 -c`
  child. In pytest they are ordinary test code that `exec`s the generated
  script's function definitions in-process (the same pinned interpreter), with
  fake `Machine` classes as plain classes, and `capsys` for the "diagnostics
  stay on stderr" assertion. Each case execs into a fresh namespace dict holding
  only the names the helpers use (`json`, `shlex`, `machine`,
  `retention_errors`, ...), never the test module's globals.
- Flake evaluations (`nix eval .#devShells` for the hooks and the `ci-nix`
  contract) are module-scoped fixtures, like the Rust `OnceLock`. They run
  before `conftest.py`'s autouse fixture redirects `HOME`, so they see the
  caller's Nix configuration. The per-case `nix-instantiate` calls need no flake
  features and run inside the tests.
- Fixture repositories are created directly under `tmp_path`, which is also the
  `GIT_CEILING_DIRECTORIES` that `conftest.py`'s autouse isolation sets, so it
  plays the Rust `parent` role. That fixture already clears `GIT_*`. The
  devShell harness itself still sets, per Bash child, everything else the Rust
  sets: `TMPDIR=tmp_path` (so "replaces inherited TMPDIR" is meaningful, not the
  caller's `.xtask/tmp`), `LD_LIBRARY_PATH` set to a sentinel or removed, and
  the `JAUNDER_TEST_*` variables. This test no longer uses the Rust
  `.xtask/issue-1811` fixture root (other users of that path are untouched).

YAML is read with PyYAML (already a `dev` dependency). PyYAML is YAML 1.1, so
the bare `on:` key loads as `True`. The Rust assertion ("`on` is a mapping under
YAML 1.2") becomes "the trigger block, loaded under the bare `on` key as `True`,
is a mapping", as `test_theme_repository_release.py` already does. It still
proves the trigger block exists and is a mapping, but it would now reject a
quoted `"on":` key, which is valid but not what the repository writes.

Prerequisites go through `conftest.require`: Nix, `jq`, `node` and the
repository's files are present on the host and in CI's host lane (where `dev`'s
`pytest` step runs), and absent only in the filtered `checks.dev-python`
sandbox, where these tests skip. This is the established pattern; no new sandbox
lane is added. Accepted cost: with `pytest -n auto`, each xdist worker that
picks up a devShell test evaluates the shells once, rather than once per test
binary.

Follow-on edits:

- `CONTRIBUTING.md`'s focused-regression command becomes
  `pytest tools/dev/tests/test_dev_shells.py`.
- `nix/dev-shells.nix`: the `devPythonCheck` comment names the pytest modules,
  not host-proof.
- `nix/checks.nix`: no Rust test embeds `ci.yml` or `performance.yml` any more,
  so `clippySourceFilter` drops both workflow admissions and their asserts.
- `tools/host-proof/Cargo.toml` drops the `serde_yaml` dev-dependency, which
  only these tests used (`repository-policy` keeps its own), and
  `tools/Cargo.lock` drops it from host-proof's entry.

## Acceptance

- The five Rust files are gone; `rg` finds no remaining reference to their names
  or to `host-proof --test dev_shell_tmpdir`.
- Each Rust `#[test]` maps to a pytest test (or parametrized set) asserting the
  same facts; the review checks this mapping file by file.
- On the host,
  `pytest tools/dev/tests/test_dev_shells.py tools/dev/tests/test_e2e_vm_script.py tools/dev/tests/test_e2e_matrix.py tools/dev/tests/test_workflow_callers.py`
  passes with no skips.
- Mutation spot-check, run once and not committed: breaking each covered
  contract (for example a hook calling `dev check`, an aggregate `if:` changed
  to `always()`, an `e2e-matrix.jq` duplicate-identity guard removed, a VM
  deadline below the floor) makes the matching pytest test fail.
- `dev check` is green, including ruff, `mypy --strict` and `checks.dev-python`
  (where the new tests skip).

## Boundaries

- No change to the contracts themselves: hooks, workflows, `nix/checks.nix`'s
  test script, `e2e-matrix.jq`, the lane catalog and devShell hooks are
  untouched beyond the comment and source-filter lines above.
- The remaining host-proof tests (`admit_inputs`, `checkout_admission`,
  `e2e_workflow`, `span_records`) stay; they test executables moved by other
  #1974 follow-ups.
- `nix/test_e2e_capture.py` stays a `unittest` module where it is; it is run,
  not rewritten.
- No new assertions beyond what the Rust tests check.
