# Issue #2134: check RustSec advisories only when dependencies change, plus a daily scan

## Outcome

A newly published RustSec advisory no longer fails, or ejects from the merge
queue, a pull request that leaves the dependency files alone. A pull request
that changes them is still checked against the current advisory database before
it lands. A daily scheduled run checks `main` and files one issue per failing
advisory.

## Approach and tradeoffs

Today two CI lanes run cargo-deny:

| Lane                      | Command                                      | Can a new advisory fail it? |
| ------------------------- | -------------------------------------------- | --------------------------- |
| Hermetic (`static-code`)  | `cargo deny check bans licenses sources`     | No: no network, no DB       |
| Host (`ci-validate-host`) | `cargo deny check` (fetches the advisory DB) | Yes                         |

The hermetic lane is unchanged and stays the bans, licenses and sources verdict
on every non-docs run
([ADR-0145](../../adr/0145-sandbox-cargo-deny-skips-advisories.md)). Only the
host lane's step changes.

- **Dependency scope.** `dev.scope` already compares a pull request or
  merge-queue entry with its base in the `merge-queue-verdict` job. It also
  prints `dependencies=true|false`: true when a changed path is the root
  `Cargo.lock`, `deny.toml`, or a `Cargo.toml` outside `tools/`. If the base or
  the diff can't be read, it prints true. A push has no base and prints false,
  because the daily scan owns `main`. `tools/` is left out because it is a
  separate workspace that cargo-deny has never checked, on the host or in the
  sandbox. Counting it would only fail tools-only bumps on product advisories
  they didn't touch. This narrows the issue's proposal, which listed
  `tools/Cargo.lock`. The missing tools advisory check is an existing gap and
  stays out of scope.
- **Host lane.** `dev ci-validate-host` takes `--advisories`, through its own
  CLI command; `_workflow` stays option-free. The workflow passes it only when
  `dependencies == 'true'`. With the flag the host lane runs full
  `cargo deny check` in its current place. Without it, the host lane has no
  cargo-deny step. Skipping loses nothing: bans, licenses and sources can't
  change without a dependency-file change, and the hermetic lane checks them
  anyway.
- **Daily scan.** A new `.github/workflows/advisories.yml` runs on a daily
  off-the-hour cron and on `workflow_dispatch`, with a non-cancelling
  concurrency group like `upgrade.yml`'s so two runs can't both file an issue.
  It checks out `main` and runs `dev advisory-scan` with `issues: write`. That
  command runs `cargo deny --format json check advisories` with `deny.toml`, so
  ignores and `unmaintained = "workspace"` behave as in the gate. It reads the
  newline-delimited JSON that cargo-deny writes to stderr and keeps
  `type == "diagnostic"` records of severity `error`, taking the ID from
  `fields.advisory.id`. For each distinct advisory ID:
  - if an **open** issue has that ID in its title, it adds a dated comment
    linking the run;
  - otherwise it opens an issue titled
    `fix(deps): <RUSTSEC-ID> <crate>: <advisory title>`, labelled `security`,
    `dependencies` and `needs-triage`. The body has the crate, version, advisory
    URL and run link.

  The run fails whenever cargo-deny fails, so a database-fetch failure with no
  advisories to parse is still a red run, just with no issue.

- **Local gates are unchanged.** `dev precommit`, `check`, `prepush` and
  `compile-checks` keep full `cargo deny check`, so a developer still sees a new
  advisory locally.
- **Records.** A new numberless ADR draft (`jaunder-adr`) records when CI checks
  advisories and cites ADR-0145, which stays accepted. The draft's projection
  updates both cargo-deny passages in `docs/ARCHITECTURE.md` ("Sandboxed
  cargo-deny" and the host-mode note). CONTRIBUTING's cargo-deny notes (around
  lines 1127 and 2078) describe the split. The `ci.yml` comments and the
  `dev/scope.py` docstring, which describe the scope as docs-only, are updated
  to match.

Rejected: `rustsec/audit-check`. It runs cargo-audit, which ignores
`deny.toml`'s ignores and unmaintained scope, so it would file issues the gate
deliberately accepts. Rejected: comparing a push with `github.event.before`.
That is a second base path, and all it gains over the daily scan is catching
admin-bypass dependency pushes up to a day sooner.

Accepted cost: an advisory published against a dependency already on `main`
reaches us through the daily issue, up to about 24 hours later, not through a
failed PR. A PR that changes dependencies can still fail on an advisory against
a crate it didn't touch. That's intended, since it is still checked before it
lands.

## Acceptance

Live proof, using the real workflow, cargo-deny and RustSec database. A base
that already contains a vulnerable crate stands in for a new advisory against an
unchanged dependency.

1. **Simulated base:** a throwaway branch from this branch whose `Cargo.lock`
   pins a crate to a version with a known advisory, such as `url` before the
   #2050 fix.
2. **PR A**, a draft against that base, changes only a `.rs` comment. Its host
   lane has no cargo-deny step, and `Validate (no e2e)` is green.
3. **PR B**, a draft against that base, makes an unrelated `Cargo.lock` change.
   Its host lane runs `cargo deny check` and fails on that advisory.
4. **Scan:** `dev advisory-scan` runs twice on the simulated base, making real
   `gh` calls against `jaunder-org/jaunder`. The first run opens one clearly
   marked test issue with the expected title and labels. The second comments on
   that issue and opens nothing.
5. **Cleanup:** close the test issue and both PRs, then delete the throwaway
   branch.

Regression locks run real code only:

- `dev.scope` tests use real git repositories, as `test_scope.py` already does.
  `dependencies` is true for a change to the root or a member `Cargo.toml`, the
  root `Cargo.lock`, or `deny.toml`, and when the base is unavailable or the
  diff is uncertain. It is false for a Rust-source-only change, a `tools/`-only
  manifest or lock change, and a push with no base. `docs_only` results don't
  change.
- The advisory parser is tested against stderr captured from the pinned
  cargo-deny on the simulated base. No `gh`-mocking test counts as proof.
- `traced-run -- dev validate-no-e2e` is green on the branch. This PR changes no
  dependency files, so its CI shows a host lane with no cargo-deny step.

Deliverables to commit: the scope, gate, CLI and workflow changes with their
tests, the ADR draft and projection, and the CONTRIBUTING updates.

## Boundaries

- No change to the hermetic `static-code` cargo-deny check, the local gates, or
  `deny.toml`'s policy.
- #1994's transitive-unmaintained review is not folded in; it may reuse
  `advisories.yml` later.
- Scan issues get labels only. The usual Backlog, milestone and Priority triage
  stays manual.
- The throwaway branch, PRs and test issue are evidence, not deliverables. They
  are closed or deleted once the proof is recorded on this PR.
