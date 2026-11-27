# ADR-0273: Pre-push takes reusable static verdicts from Nix

- Status: accepted
- Date: 2026-11-26
- Issue: [#2136](https://github.com/jaunder-org/jaunder/issues/2136)

## Context

`dev prepush` ran its whole Verify host surface on the host, except that on
x86_64-linux the Markdown slot already took the Nix `static-docs` verdict
([ADR-0029](0029-git-enforced-verify-gate.md), #1863 supplement). In local
traces of 14 pre-push runs (2026-11-22 to 26), the median run took 694 s:

- `test-local`: 374 s
- auxiliary host tests: 126 s, of which `pytest` was 99 s
- host Clippy: 73 s
- formatting, `tsc` and Elisp: about 36 s together

Only one of those 14 runs re-pushed a tree that had already been pushed. The
reuse available is therefore across trees, not of identical trees.

A Nix check is input-addressed and reads a filtered source slice. When a branch
leaves every input of a check untouched, the derivation is the one `main` built.
It is then reused from the local store, or substituted from Cachix, which CI
fills under the static-verdict rule of
[ADR-0267](0267-ci-honours-non-substitutable-verdicts.md).

Alternatives were measured and rejected:

- **Nix Clippy.** It re-lints every workspace crate from scratch on any Rust
  change: about 164 s with dependencies cached, and over 400 s when they
  rebuild.
- **A Nix or dropped `test-local`.** A Nix test derivation rebuilds from cold on
  every Rust change. Dropping it moves defect discovery to CI and stops the
  hooks exercising host-only build paths
  ([ADR-0255](0255-ci-clippy-verdict-from-nix.md)).
- **`dev-python`.** Its source is only `tools/dev`, so about 25 tests that check
  `dev` against hooks, workflows, `flake.nix` and scripts skip there. Keeping
  those tests on the host would still cost 58 s of the 99 s.
- **Pre-commit requesting the same derivations.** Replaying 39 real pushes, it
  cost about 155 s of pre-commit work per push to save at most 95 s of pre-push
  work.

## Decision

On x86_64-linux, `dev prepush` takes these Nix checks in place of the host
leaves they cover:

| Nix check           | Host leaves it replaces         |
| ------------------- | ------------------------------- |
| `rust-format`       | `fmt`, `leptosfmt`, `tools-fmt` |
| `static-docs`       | `prettier-markdown`             |
| `static-elisp`      | `elisp-fmt`, `byte-compile`     |
| `static-typescript` | `tsc`                           |
| `static-ert`        | `ert`                           |

Each check is requested at the slot of the first leaf it covers, so the
fail-fast order is otherwise unchanged. Clippy, the other host leaves, the
auxiliary host tests, `test-local` and the Nix doctests stay as they were. Other
platforms run the host leaves.

Index custody covers the whole of pre-push's checks, not one docs request:

- After the clean-tree refusal, one admission requires every tracked regular
  file's raw worktree bytes to equal its index blob. Symlinks and gitlinks are
  not hashed.
- At the end, one post-check requires Git state to be unchanged.
- The post-check also runs after a failed step. When it passes, that step's
  status is the gate's; when it fails too, the gate refuses and names both.
- A signal skips the post-check.

The Markdown pre-commit route uses the same whole-tree identity and the same
custody, so a failed `static-docs` request there now exits with its own status
rather than 1. The hand-kept list of `static-docs` inputs is gone.

## Consequences

- A push that leaves `elisp/`, `end2end/`, the Rust sources and `dev` untouched
  takes those verdicts from the store, saving about 20–30 s. A push that changes
  `dev`, an input of every check, rebuilds them at about the host cost.
- Locally, `tsc` and the Elisp checks no longer run on the host on x86_64-linux.
  CI's host lane still runs them.
- A raw/index mismatch on any tracked file, not only Markdown, now refuses
  pre-push and the Markdown pre-commit route.
- Custody now spans the whole push, about ten minutes, not one docs request.
  Editing a tracked or nonignored file while a push runs fails the push.
- The issue's "re-push under a minute" target is not met, by decision: host
  `nextest`, Clippy and source contracts still run on every push.
- This amends the #1117 and #1863 supplements of
  [ADR-0029](0029-git-enforced-verify-gate.md). Current state:
  `docs/ARCHITECTURE.md` §The verify ladder & git-enforced gate.
