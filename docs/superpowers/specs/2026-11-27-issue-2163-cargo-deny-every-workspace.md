# Issue #2163: cargo-deny checks every Cargo workspace

## Outcome

Every lane that runs cargo-deny checks both Cargo workspaces, the root product
workspace and `tools/`, under the one root `deny.toml`. An advisory, banned
crate, disallowed license or disallowed source in `tools/` is caught where the
same problem in the product would be.

## Approach and tradeoffs

Today cargo-deny runs only `cargo deny check` from the root, which resolves the
product workspace alone. `tools/` is a separate workspace that no lane checks,
so `zip` 9.0.4 in `tools/Cargo.lock` (RUSTSEC-2026-0548, Dependabot alert #4)
has gone unreported by our own gates since October. Measured on `main`, `tools/`
passes bans, licenses and sources; its only failure is that advisory.

| Where                      | Product workspace (today) | Adds for `tools/`               |
| -------------------------- | ------------------------- | ------------------------------- |
| Hermetic `static-code`     | bans, licenses, sources   | bans, licenses, sources         |
| Local gates (hooks)        | full `cargo deny check`   | full check, advisories included |
| CI host lane, deps changed | full `cargo deny check`   | full check, advisories included |
| Daily `advisory-scan`      | advisories                | advisories                      |

- **One new static leaf.** `dev static tools-cargo-deny`, beside `tools-fmt` and
  `tools-clippy`, runs `cargo deny --manifest-path tools/Cargo.toml check` on
  the host and the offline subset in `--sandbox` against the tools offline Cargo
  home `static-code` already provides. It runs wherever `cargo-deny` runs:
  `static-code` hermetically, the CI host lane with `--advisories`, and locally
  in `compile-checks` and `static-all`. Locally it runs last in
  `compile-checks`, after the Clippy units, so its red window (below) hides as
  little as possible.
- **Dependency scope covers every workspace.** `dev.scope` counts any changed
  `Cargo.toml` or `Cargo.lock`, in any directory, and `deny.toml` as a
  dependency change. This is the rule #2134's issue first proposed; #2134
  narrowed it only because `tools/` was unchecked.
- **The daily scan checks both workspaces.** `dev advisory-scan` runs the
  advisory check once per workspace and reports each advisory ID once. The issue
  body lists every place it was found (crate, version and workspace), and the
  run fails if either check fails.
- **Records.** #2134's decision is being promoted to ADR-0275 by the open
  promoter PR #2170. That ADR is not edited. A new numberless ADR draft amends
  it: `tools/` is no longer outside cargo-deny, and its "may fail on a crate it
  didn't touch" consequence now covers both workspaces. The branch rebases onto
  #2170 once it lands, and the draft links the numbered ADR. Every place that
  says "product dependencies only" or "outside `tools/`" is updated:
  ARCHITECTURE, CONTRIBUTING, the `dev.scope`, `dev.advisories` and
  `ci_validate_host` docstrings, the `--advisories` help text, the `ci.yml`
  comments, and the `advisories.yml` header.

Rejected: a separate `tools/deny.toml`. The tools workspace already passes the
root policy, and two policy files could drift apart.

### The deliberate red window

`zip` stays at 9.0.4 so that the first scan after merge files a real issue for
RUSTSEC-2026-0548 from inside Actions, which nothing has proved yet. So:

- **On this branch.** Every commit except the last goes through its hooks
  normally. The last commit adds `tools-cargo-deny` to the local gates. A gate
  stops at its first failure, so from that commit on the hooks fail at that
  leaf, and the checks after it never run: the Elisp checks and auxiliary host
  tests, plus `test-local` and doctests on push. For that one commit and its
  push:
  1. The hook runs first, and its only failure must be RUSTSEC-2026-0548 on
     `zip` in `tools/`.
  2. The checks the failure hides then run separately on the same tree:
     `dev elisp-checks`, the auxiliary host tests, `dev test-local` and
     `dev doctests`.
  3. Only then is it committed and pushed with the hook skipped. The PR body
     records the results.
- **This PR's CI stays green.** It changes no dependency file, so the host lane
  skips advisories, and the hermetic lane checks no advisories.
- **After merge, until `zip` is upgraded:**
  - Everyone's local hooks fail at the same leaf, with the same hiding.
  - Every PR or merge-queue entry that changes any `Cargo.toml` or `Cargo.lock`,
    Dependabot's included, fails its host lane on RUSTSEC-2026-0548.

  The human times the merge to limit both. The `zip` upgrade follows straight
  away as its own change, and it passes because it removes the advisory.

## Acceptance

Proof to run:

- `dev.scope` tests on real git repositories: `dependencies=true` for a
  `tools/Cargo.lock` change and a `tools/<member>/Cargo.toml` change, alongside
  the existing product and `deny.toml` cases.
- Gate and static tests that fix the leaf set and command order:
  `test_static.py` (`SANDBOX_CARGO` and `LEAVES`), `test_gates.py`, and
  `test_workflows.py`'s `STATIC_ALL`. They show `tools-cargo-deny` last in
  `compile-checks`, in `static-all`, and in the CI host lane with
  `--advisories`, and running `bans licenses sources` with
  `--manifest-path tools/Cargo.toml` in sandbox mode. `test_scope.py`'s `tools/`
  exclusion cases become dependency changes.
- `dev advisory-scan` tests. The `zip`-in-`tools/` report is captured from the
  pinned binary on `main`. The same-ID-in-both-workspaces report is built from
  captured records, since no advisory hits both today. The tests cover one
  advisory in one workspace, one ID in both, and a failure in either workspace.
- On this branch: `dev static tools-cargo-deny` fails on RUSTSEC-2026-0548
  alone, and `dev nix-check static-code` passes.
- PR CI green.
- **After merge,** dispatch `advisories.yml` on `main` twice. If the daily cron
  runs in between, that run counts as one of the two.
  - Exactly one open issue for RUSTSEC-2026-0548 exists, opened by Actions and
    naming `zip` 9.0.4 in `tools/`.
  - The later run comments on it and opens nothing new.
  - The Lead records both run links on #2163, since the PR is closed by then.

## Boundaries

- No `zip` upgrade here; it follows the post-merge scan.
- No change to `deny.toml`'s policy, and no `tools/deny.toml`.
- Other Cargo projects outside both workspaces (for example `vendor/`) are out
  of scope; they are patched into the product workspace, not built alone.
