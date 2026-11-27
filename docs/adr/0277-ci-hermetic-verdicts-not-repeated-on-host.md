# ADR-0277: CI does not repeat hermetic verdicts in the host lane

- Status: accepted
- Date: 2026-11-26
- Issue: [#2144](https://github.com/jaunder-org/jaunder/issues/2144)

## Context

[CI takes its Clippy verdict from Nix](0255-ci-clippy-verdict-from-nix.md)
removed host Clippy from `dev ci-validate-host` because the hermetic lane's
`static-code` already decided it. The other host steps with a hermetic twin were
never revisited, so CI still decided each of them twice:

| Host step                                                              | Hermetic check      |
| ---------------------------------------------------------------------- | ------------------- |
| `fmt`, `leptosfmt`, `tools-fmt`                                        | `rust-format`       |
| `prettier-markdown`                                                    | `static-docs`       |
| `prettier-end2end`, `prettier-css`, `ast-grep-tests`, `no-full-reload` | `static-code`       |
| `tsc`                                                                  | `static-typescript` |
| `elisp-fmt`, `byte-compile`                                            | `static-elisp`      |
| `ert`                                                                  | `static-ert`        |
| `ruff-format`, `ruff-check`, `mypy`, `pytest`                          | `dev-python`        |

Across the 68 CI runs measured on #2144 these steps took a median of 176 s of
the host lane's 258 s of `dev` time. The host lane was rarely the last lane to
finish, so the cost was runner time, not wall clock. The larger cost was
ambiguity: two lanes could disagree, and "which lane decides this?" had no
single answer.

`dev-python` could not simply take over `pytest`. Its filtered `tools/dev`
source and sandbox lack the repository's other files, jq and Nix, so about 230
tests skipped there (`require()` in `tools/dev/tests/conftest.py`), and only the
host run executed them.

## Decision

A check whose CI verdict a hermetic-lane derivation owns is not repeated in the
host lane. `dev ci-validate-host` runs only what Nix cannot check, or does not:

- the docs-shape checks and the `source-contracts` checks;
- the platform contract;
- the tools Cargo tests (which also keep the lane as `main`'s kache writer);
- the `repository` tests: `pytest -m repository tools/dev`;
- `cargo-deny`, for its RustSec advisories, which the sandboxed copy skips
  ([ADR-0145](0145-sandbox-cargo-deny-skips-advisories.md)), only on a
  dependency change
  ([CI checks advisories only on a dependency change](0276-ci-checks-advisories-on-dependency-change.md)).

The `dev` tests are split by a pytest marker, not by skipping. A test that needs
the repository outside `tools/dev`, jq or Nix is marked `repository`;
`dev-python` runs `pytest -m "not repository"` and the host lane runs exactly
the marked set. `require()` always fails on a missing prerequisite. An unmarked
test that needs the repository therefore fails `dev-python`, and a mark on a
test `dev-python` needs drops its code from `dev-python`'s 100% coverage floor,
which fails too. Every `dev` test runs in exactly one CI lane.

Local gates are unchanged: `dev check`, `dev check-no-test`, the hooks and local
`dev validate-no-e2e` keep every host step, so `validate-no-e2e` stays a
superset of the CI lanes.

This generalises ADR-0255, whose note that the host lane "still runs `tsc` and
`cargo-deny`" no longer holds for `tsc`, and narrows the `host` lane of
[distributed CI lane topology](0203-distribute-ci-validation-and-firefox-e2e-lanes.md)
further: the hermetic lane owns each verdict it can decide.

## Consequences

- About 3 runner-minutes saved per non-docs CI run; no measurable wall-clock
  change.
- A CI formatting, TypeScript, Elisp, ERT or `dev` lint/type failure appears in
  the hermetic lane, against that check's filtered source.
- When `tools/dev` is unchanged, `dev-python` is substituted from Cachix, so CI
  does not rerun the non-`repository` tests for that input. That is the same
  verdict for the same derivation
  ([substitutable static verdicts](0252-substitutable-static-verdicts.md)).
- A new host step with a hermetic twin must not be added to `ci-validate-host`;
  `tools/dev/tests/test_gates.py` pins the dropped steps against the owning
  derivations in `nix/checks.nix`.
