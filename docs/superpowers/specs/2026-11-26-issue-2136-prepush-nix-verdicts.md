# Issue #2136: pre-push takes cacheable static verdicts from Nix

## Outcome

On x86_64-linux, `dev prepush` gets its Rust formatting, TypeScript and Elisp
verdicts from Nix flake checks instead of host runs. Their source slices are
narrow, so a push often finds a matching verdict already in the store, from this
machine or from CI through Cachix, and gets it in seconds. The checks pre-push
requires are the same. Clippy, `test-local` and the host Python tests stay on
the host, by measured decision.

## Measurements behind the decisions

Local traces (14 pre-push runs, 2026-11-22..26) and Nix build logs:

| Pre-push part              | Host median | Nix alternative, when built                |
| -------------------------- | ----------- | ------------------------------------------ |
| whole pre-push             | 694 s       | issue baseline: 636 s over 148 runs        |
| `test-local`               | 374 s       | none cheaper (coverage takes about 13 min) |
| auxiliary host tests       | 126 s       | `pytest` 99 s; see `dev-python` below      |
| host Clippy (six units)    | 73 s        | 164 s to over 400 s                        |
| source contracts           | 32 s        | no Nix leaf                                |
| Elisp byte-compile and ERT | 27 s        | `static-elisp` 1.5 s + `static-ert` 20 s   |
| format group, `tsc`        | 9 s         | `rust-format` 5 s, `static-typescript` 1 s |

Only 1 of 14 pushes re-pushed an already-pushed tree, so the gain comes from
reuse across trees. Each Nix check reads a filtered source slice plus its tool
inputs:

- All of them take the `dev` package.
- `rust-format` also takes the `tools/` workspace (`repository-policy`), the
  toolchain and the offline Cargo homes.
- `static-typescript` also takes the whole-`end2end` `e2ePackage`.

A branch that leaves all of a check's inputs untouched gets `main`'s derivation.

## Approach and tradeoffs

On x86_64-linux, four host steps of pre-push become Nix checks, each at the slot
of the first host step it replaces:

| Nix check           | Replaces                        | Slot        |
| ------------------- | ------------------------------- | ----------- |
| `rust-format`       | `fmt`, `leptosfmt`, `tools-fmt` | `fmt`       |
| `static-elisp`      | `elisp-fmt`, `byte-compile`     | `elisp-fmt` |
| `static-typescript` | `tsc`                           | `tsc`       |
| `static-ert`        | `ert`                           | `ert`       |

`static-docs` already replaces `prettier-markdown` at its slot. Every other step
keeps its place and runs on the host.

The `static-docs` custody becomes pre-push-wide:

- **One admission, right after the clean-tree refusal.** The tree must be
  provably clean. Every tracked regular file (modes 100644 and 100755) must have
  raw worktree bytes equal to its index blob (`git hash-object --no-filters`);
  symlinks and gitlinks are not hashed.
  - This checks about 1,750 files, 24 MB, in one call.
  - It replaces the hand-kept `static-docs` input filter, which mirrored a Nix
    source filter.
  - The narrow Markdown pre-commit route uses the same whole-tree identity.
    `.gitattributes` has no conversions, so its behaviour is unchanged today.
- **One post-check, at the end of pre-push.** Git state must be unchanged since
  the admission.
  - It also runs after a failed step, and a failure there never hides that
    step's own failure.
  - It is skipped on a signal, as today.
- **Phases and requests.** The phases are `prepush-admission` and
  `prepush-post-check`, replacing the `static-docs-*` names. Requests keep the
  existing locked-flake flags and the gates' stop-at-first-failure rule.

Other platforms keep the host steps, because `dev nix-check` evaluates only
`checks.x86_64-linux`. `check`, `check-no-test`, `precommit`'s broad route,
`validate-no-e2e` and the CI lanes are unchanged.

**Rejected alternatives**, measured:

- **Nix Clippy.** A Rust change re-lints every workspace crate from scratch.
- **Nix or dropped `test-local`.**
  - A Nix test derivation rebuilds from cold on every Rust change.
  - Dropping it moves defect discovery to CI, about 14 min later, and stops the
    hooks exercising host-only build paths (ADR-0255).
- **`dev-python`.**
  - About 25 tests check `dev` against hooks, workflows, `flake.nix` and
    scripts. Many reach these through fixtures. They skip in the Nix check,
    whose source is only `tools/dev` (`tools/dev/tests/conftest.py:110-121`).
  - Keeping those tests on the host still costs 58 s of `pytest`'s 99 s.
  - Every `dev` edit would also cost a 132 s build.
- **Pre-commit requesting the same derivations.** Replaying 39 real pushes, this
  costs about 155 s of pre-commit work per push to save at most 95 s.
- **Narrowing the `dev` input**, so a `dev` edit doesn't invalidate every check.
  That is new and fragile machinery.

**Accepted cost.** The expected saving is modest:

- Pushes that leave a check's inputs untouched save about 20–30 s, for example
  Elisp and ERT when neither `elisp/` nor `dev` changes.
- Pushes that change `dev` rebuild all four checks, which costs about the same
  as the host steps.
- Locally, `tsc` and the Elisp checks no longer run on the host. CI's host lane
  still runs them.

**Record.** A new ADR draft amends ADR-0029's #1117 and #1863 supplements.
Pre-push requests these substitutable static verdicts under ADR-0267's rule,
with pre-push-wide custody. The same commit updates:

- `docs/ARCHITECTURE.md`
- CONTRIBUTING: the hook paragraphs (around lines 206–243), the gate table,
  "Prepush parity by failure surface", and the family-decision bullets for
  Markdown, other formatting and Elisp
- the `nix/packages.nix:821-823` comment that pairs `static-docs`'s filter with
  `_static_docs_input` (a comment-only change)

## Acceptance

The issue's "re-push under a minute" and "after median over 20 pushes" criteria
are changed by human decision (2026-11-26). Host `nextest`, Clippy and source
contracts stay, and only 1 of 14 pushes repeats a tree.

Proof to execute, with results reported in the PR:

- **`dev` tests.** `tools/dev` pytest at its 100% coverage gate covers:
  - the slot replacements and the non-x86 host fallback
  - the whole-tree admission, including a refused raw/index mismatch on a
    non-Markdown file and the ignoring of symlinks and gitlinks
  - the post-check, after success and after a failed step
- **First run.** A traced `dev prepush` on this branch passes.
- **Re-run.** A second run on the same tree shows all five Nix steps `reused`.
  The PR reports both runs' totals and the five steps' eval and build time.
- **Unchanged slice.** A traced `dev prepush` on a tree that touches neither
  `elisp/` nor `dev` shows `static-elisp` and `static-ert` `reused` or
  `substituted`.
- **Medians.** The PR records the before median (636 s over 148 runs). A
  follow-up issue records the after median once 20 real pushes exist.

Deliverables to commit: the `dev` change and its tests, the ADR draft, and the
docs listed under Record.

## Failures the issue asked about

- **Explained.** The no-change repeats in #2020 and #1956 were:
  - #2048: kache served a `jaunder` build that read a removed checkout's
    directory, fixed by PR #2054
  - RUSTSEC-2026-0592: fixed by #2049 and PR #2050
- **Recorded only.** The 30 s `code_highlight` nextest timeouts cleared on
  re-run and coincided with concurrent pre-pushes in other checkouts. No issue
  is filed.

## Boundaries

- No change to Clippy, `test-local`, the host Python tests, pre-commit's broad
  route, `check` or `validate*`, the CI lanes, or Nix check definitions and
  source filters.
- No bespoke "already checked" stamp. The Nix store is the only reuse.
