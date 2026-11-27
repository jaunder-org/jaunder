# ADR-0275: CI checks advisories only on a dependency change, plus a daily scan

- Status: accepted
- Date: 2026-11-26
- Issue: [#2134](https://github.com/jaunder-org/jaunder/issues/2134)

## Context

cargo-deny runs in two CI lanes. The hermetic lane's `static-code` check runs
`cargo deny check bans licenses sources` with no network and no advisory
database ([ADR-0145](0145-sandbox-cargo-deny-skips-advisories.md)). The host
lane runs full `cargo deny check`, which fetches the RustSec database. It ran on
every pull request and merge-queue run.

So every RustSec publication against a dependency already on `main` failed every
queued entry, whether or not the entry touched dependencies. From 2026-11-12 to
2026-11-26, four merge-queue ejections came from new advisories on unchanged
dependencies, and advisories caused six of the host lane's nine infrastructure
failures. The fix (a bump or an ignore) was always a separate pull request. The
human decision on 2026-11-26 was to change when advisories are checked, not
whether.

## Decision

The CI host lane runs cargo-deny, advisories included, only for a pull request
or merge-queue entry that changes the product workspace's dependencies relative
to its base. Those are the root `Cargo.lock`, `deny.toml`, or a `Cargo.toml`
outside `tools/`. If the base or the diff can't be read, the entry counts as a
dependency change. CI's merge-queue verdict job decides this alongside the
documentation-only scope (`dev.scope`) and passes `--advisories` to
`dev ci-validate-host`. Without it, the host lane has no cargo-deny step. The
hermetic `static-code` check stays the bans, licenses and sources verdict on
every run: those can't change without a dependency-file change.

A push to `main` never checks advisories. A daily scheduled workflow
(`.github/workflows/advisories.yml`, `dev advisory-scan`) runs
`cargo deny check advisories` on `main` with `deny.toml`, so its ignores and
unmaintained scope are the gate's. For each failing advisory, it comments on the
open issue whose title carries the advisory ID, or opens one labelled
`security`, `dependencies` and `needs-triage`. The run fails whenever cargo-deny
fails.

Local gates (`dev precommit`, `check`, `prepush`, `compile-checks`) keep full
`cargo deny check`, so a developer still sees a new advisory locally.

## Consequences

A newly published advisory against an unchanged dependency fails no pull request
and ejects nothing. It reaches the tracker within a day as an issue. A pull
request that changes dependencies is still checked against the current database,
and can still fail on an advisory against a crate it didn't touch.

The `tools/` workspace is outside cargo-deny in every lane, so its changes don't
trigger the check and its advisories go unreported. That gap predates this
decision.

Rejected: `rustsec/audit-check`. It runs cargo-audit, which ignores
`deny.toml`'s ignores and unmaintained scope, so it would file issues the gate
accepts. Rejected: comparing a push with the previous `main` tip. That is a
second base path, and all it adds over the daily scan is earlier detection of an
admin-bypass dependency push.

The periodic review of transitive unmaintained advisories (#1994) may reuse the
scheduled workflow.
