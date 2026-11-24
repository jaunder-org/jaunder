# ADR-0265: An upgrade test replaces deployment qualification

- Status: accepted
- Date: 2026-11-24
- Issue: [#2069](https://github.com/jaunder-org/jaunder/issues/2069)

## Context

`tools/qualification` (about 8,300 lines) proved deployments by booting QEMU,
Caddy and `curl` probes by hand. It linked product code to learn facts the
deployed server already exposes, and it carried three synthetic styling builds
(`a`, `b-app`, `b-theme`). [ADR-0261](0261-tool-language-by-maintainability.md)
retired it in favour of a scheduled CI upgrade test. That ADR also left four
choices to #2069: the earlier build to upgrade from, what to compare, where
comparisons live, and whether one test can deploy two immutable revisions.

The question the test answers is whether a site an earlier Jaunder wrote still
works after an operator upgrades it. Pixel or byte goldens would fail on every
intended change, so they cannot answer it. Running the ordinary e2e suite on an
upgraded site cannot answer it either, because each test provisions its own data
and never reads the earlier records.

Restore requires an exact schema match
([ADR-0174](0174-backup-format-and-schema-compatibility.md)), so a backup cannot
carry earlier data across a migration. Opening an existing database runs pending
migrations, which is what a package upgrade does.

## Decision

**Create, verify, upgrade, verify.** One NixOS VM test per backend (SQLite and
PostgreSQL) runs four steps:

1. The earlier build's own `jaunder`, `test-support` and e2e package run the
   existing `createProductionBaseline` as the service user. That covers the demo
   sandbox profile plus browser- and AtomPub-created posts, media, App
   Passwords, sessions and subscriptions.
2. The earlier build then verifies those records with
   `verifyProductionBaseline`. Only earlier binaries touch the database in these
   first two steps.
3. `switch-to-configuration test` moves the node to a specialisation that
   differs only in `services.jaunder.package`. The service stops gracefully, and
   the current build's `preStart` migrates the database in place.
4. The current build's e2e package verifies the same records, starting from the
   author's saved browser session. It then mints and revokes an App Password to
   show new writes work.

There are no goldens and no second assertion suite. The ordinary
`production-baseline-flow` spec keeps proving that create and verify agree
within one build.

**Which earlier build.** `upgrade.yml` runs on three triggers:

- a `v*` tag push, upgrading from the previous tag;
- a weekly cron, upgrading from the head commit of the newest successful run of
  itself on `main`;
- a manual dispatch with an explicit `from`.

With no such build, a run upgrades from itself.
`dev upgrade-test --from <rev> <backend>` runs one backend locally, and
`dev upgrade-dispatch` starts the workflow. The earlier flake is loaded with
`builtins.getFlake` on the checkout's own Git objects. The main flake gains no
input, and ordinary CI never builds an earlier revision.

**The cross-version contract.** The state the earlier build writes and the demo
manifest it carries are read by a later build. `BaselineState` has an explicit
version. Readers accept every state and manifest version written since the
latest tag, and they prune older versions only after a newer tag's upgrade run
is green.

**What changes in the ADRs this touches.**

- This decision supersedes [ADR-0239](0239-qualification-workflow-ownership.md).
- It settles the four choices
  [ADR-0261](0261-tool-language-by-maintainability.md) deferred, replacing its
  golden-file and retained-browser wording.
- It removes the qualification provenance from
  [ADR-0225](0225-nix-cached-host-tooling.md).
- It removes the qualification carveout from
  [ADR-0247](0247-host-tool-json-report-contract.md).
- It removes the `production-baseline-*` commands from
  [ADR-0251](0251-dev-runner-replaces-just.md).
- It removes `qualification-host` from the Nix-built host tools of
  [ADR-0258](0258-dev-runs-nix-built-host-tools.md).

## Consequences

- `tools/qualification`, the `production-baseline-*` commands, the
  production-baseline VM module and packages, its runbook and schema, and the
  `qualification` Cargo features are gone.
- The styling fixtures shrink to the `test-utils` functions
  `host::system_theme::test_variants::{changed_application, changed_studio}`,
  used by storage and server unit tests.
- Accepted losses:
  - The current NixOS module runs both builds, so breakage inside the module
    itself is not covered.
  - No browser is retained across the upgrade, and nothing probes that the
    earlier build's stylesheet URLs still resolve.
  - Cross-backend restore after an upgrade is left to the backup interop tests.
  - Coverage is whatever `verifyProductionBaseline` reads.
- A tag run reports after the tag exists. A release is checked before tagging by
  dispatching with `from=<last tag>`.
- Changing the baseline state or the demo profile carries a reader for the
  previous version until the next tag.
