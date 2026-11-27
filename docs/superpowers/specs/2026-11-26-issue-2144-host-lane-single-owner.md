# Issue #2144: the host lane drops checks the hermetic lane owns

## Outcome

In CI, each check the hermetic lane (`dev ci-validate-hermetic`) already decides
gets its verdict from that lane only. `dev ci-validate-host` stops repeating
those checks. Every `dev` test runs in exactly one CI lane, and a test that
lands in neither lane fails the build loudly.

## Approach and tradeoffs

**Host lane step list.** `ci_validate_host` (`tools/dev/dev/gates.py`) becomes:

1. clean tree
2. docs-shape checks (`DOCS_SHAPE`)
3. the `source-contracts` checks (`SOURCE_CONTRACTS`), without `ast-grep-tests`
   and `no-full-reload`, which `static-code` runs
4. `cargo-deny` (still the only RustSec advisory scan until #2134 moves it)
5. platform contract
6. tools Cargo tests (`tools-tests-build`, `tools-tests`), which keep the lane
   as `main`'s kache writer
7. `pytest -m repository tools/dev`, run as the step `pytest-repository`

It drops these steps; the hermetic check that owns each is in parentheses:

| Dropped steps                                                          | Owning hermetic check |
| ---------------------------------------------------------------------- | --------------------- |
| `fmt`, `leptosfmt`, `tools-fmt`                                        | `rust-format`         |
| `prettier-markdown`                                                    | `static-docs`         |
| `prettier-end2end`, `prettier-css`, `ast-grep-tests`, `no-full-reload` | `static-code`         |
| `tsc`                                                                  | `static-typescript`   |
| `elisp-fmt`, `byte-compile`                                            | `static-elisp`        |
| `ert`                                                                  | `static-ert`          |
| `ruff-format`, `ruff-check`, `mypy`, the full `pytest`                 | `dev-python`          |

**When each lane runs.** `validate-host` runs on every run that isn't docs-only.
`validate-hermetic` also skips a `main` push whose SHA already passed the merge
queue (`tested`). That SHA got its hermetic verdict in the queue, and a failed
verdict job fails `Validate (no e2e)` anyway. So the hermetic lane decides every
commit's verdict for these checks.

Local gates (`check`, `check-no-test`, `precommit`, `prepush`,
`validate-no-e2e`) keep their host steps unchanged. The `.#ci` shell stays as it
is, because the performance and mutants jobs share it.

**Splitting the `dev` tests between lanes.** Some `dev` tests need repository
files outside `tools/dev`, jq or Nix. `dev-python`'s filtered source and sandbox
don't have them. Today `require()` (`tools/dev/tests/conftest.py`) skips those
tests when `JAUNDER_DEV_PYTHON_CHECK=1`. I measured 234 of 1728 on a copy of
`tools/dev`, all for missing files. Some need Nix, so the sandbox can't run them
all. The split becomes explicit:

- A pytest marker, `repository`, is registered in `tools/dev/pyproject.toml`,
  and every test that skips in `dev-python` today carries it. Mark per test
  where a module mixes the two kinds. The twiggy test in `test_wasm_audit.py`
  isn't marked, because twiggy is in the sandbox.
- `dev-python` runs `pytest -m "not repository"`. The host lane runs
  `pytest -m repository`.
- `require()` always fails on a missing prerequisite. The
  `JAUNDER_DEV_PYTHON_CHECK` variable and its skip branch are removed.
- So a test that needs the repository but isn't marked fails in `dev-python`,
  and a marked test fails in the host lane if a prerequisite is missing.
  Over-marking drops `dev` code out of `dev-python`'s coverage, so its 100%
  floor fails. Skipped tests never counted toward that floor, so the floor
  itself is unchanged.

Rejected: an autouse fixture that makes `require()` refuse unmarked skips. It
misses `require()` calls in module-scoped fixtures (`test_dev_shells.py`,
`test_e2e_vm_script.py`), and it keeps the skip path. Rejected: keeping the full
host `pytest`. It's 137 s of the 176 s of duplicated work.

**Ownership rule.** A new ADR draft (through `jaunder-adr`) generalises
ADR-0255: "a check whose CI verdict the hermetic lane owns isn't repeated in the
host lane." The host lane keeps:

- the docs-shape checks and the source contracts;
- the platform contract;
- the tools Cargo tests;
- the `repository` tests;
- `cargo-deny` advisories, until #2134 moves them.

The ADR is projected into `docs/ARCHITECTURE.md`. CONTRIBUTING's host-lane
description (about line 1047) changes. "Prepush parity by failure surface" needs
no change, because neither `prepush` nor `validate-no-e2e` changes. #2140 is
still open, so it has no layer document to update.

## Acceptance

- `test_gates.py` pins `ci-validate-host`'s exact command list.
- A `repository`-marked test reads `nix/checks.nix` and checks the mapping
  table. For each dropped step, the owning derivation's text runs it
  (`dev static <step> --sandbox`, or ruff, mypy and `pytest -m "not repository"`
  for `dev-python`). The test also checks that `ci-validate-host` runs none of
  the dropped steps and that each owner is in `STATIC_CHECKS`.
- `test_prerequisites.py`: `require(False, …)` fails whatever the environment.
- Building `.#checks.x86_64-linux.dev-python` passes, and on the host
  `pytest -m repository tools/dev` passes with no skips.
- `dev check-no-test` passes.
- After merge, the host lane trace of one merge-queue run shows the reduced step
  list and its `dev` time. That evidence is posted on #2144.

## Boundaries

- `cargo-deny` stays in the host lane until #2134.
- The only Nix change is `dev-python`'s pytest selection and the removed
  environment variable. No change to the local gates, the hooks, the `.#ci`
  shell or `ci-validate-docs`.
- Local pre-commit `pytest` cost is out of scope.
