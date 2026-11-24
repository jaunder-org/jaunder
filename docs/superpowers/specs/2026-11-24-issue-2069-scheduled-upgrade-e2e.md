# Issue #2069: replace qualification with a scheduled upgrade test

## Outcome

Once a week, CI checks that a site written by an earlier Jaunder survives an
in-place upgrade to the current commit, on SQLite and PostgreSQL.

1. The earlier build creates a representative site.
2. The earlier build verifies that site.
3. The service is upgraded in place to the current build.
4. The current build verifies the same records.

It also runs on each `v*` tag push, upgrading from the previous tag, which is
the release-to-release path. The weekly run upgrades from the commit of the last
green run, which gives early warning between releases. `tools/qualification` and
everything that exists only for it is deleted.

## Approach and tradeoffs

**The assertions already exist.** `end2end/tests/production-baseline.ts` is a
create/verify pair, and ordinary e2e runs it today through
`production-baseline-flow.spec.ts`.

- `createProductionBaseline` seeds the demo sandbox profile through
  `test-support`. That profile has four users, posts in every visibility,
  Markdown and Org posts, and media. It then creates further records through the
  browser and AtomPub: Markdown, Org and HTML posts, scheduled and
  subscribers-only posts, AtomPub media, app passwords, sessions and
  subscriptions. It returns a JSON-serialisable `BaselineState`.
- `verifyProductionBaseline(state)` checks every one of those records:
  - permalinks and rendered text;
  - audience gating, for subscriber and non-subscriber sessions;
  - AtomPub members;
  - media bytes against their sha256;
  - feeds;
  - the seeded manifest.

The upgrade test runs these halves on different builds. It adds no new
assertions and no golden files.

**One VM test per backend** (`mkUpgradeCheck`). It reuses `mkE2eCheck`'s node,
collector, capture directory and backend setup through its existing `producer`
hook in `nix/checks.nix`, which perf already uses. The Playwright invocation in
`e2eRunAndCapture` is factored into a fragment that the producer runs twice,
once per phase. Each phase runs in its own e2e package directory, and both share
a state path. Capture lift, the panic gate and the assertions run once, at the
end. Only Chromium runs, as a single Playwright project.

1. **Earlier build: create, then verify.**
   - **Earlier binary.** The NixOS module gains a standard
     `services.jaunder.package` option (replacing its factory argument); the
     base node sets it to the earlier `jaunder`.
   - **No current binaries.** The earlier `jaunder` and `test-support` lead
     `PATH`. No current-build binary touches the database in this phase. This
     matters because opening a database migrates it. The phase runs as the
     service user, so the files its seed tools create (database, Media, the
     publisher lock) stay openable by the service.
   - **Earlier toolchain.** The earlier `e2e-support` package runs under the
     earlier nixpkgs' `nodejs` and `playwright-driver.browsers`, so a Playwright
     bump inside the window can't break this phase.
   - **What the spec does.** `upgrade.spec.ts` (phase `baseline`) applies the
     flow spec's site-config setup, through a helper extracted from that spec
     rather than a copy. It runs `createProductionBaseline`, then
     `verifyProductionBaseline`. It writes `BaselineState` to the guest, along
     with the browser's Playwright `storageState`: the author's logged-in
     session, which `verifyProductionBaseline` relies on.
   - A failure here belongs to the earlier build, not to the upgrade.
2. **Upgrade in place.**
   - `switch-to-configuration test` moves the node into a specialisation that
     differs only in `services.jaunder.package`.
   - systemd stops `jaunder` gracefully, as in production (ADR-0250).
   - It then runs the current `preStart` (`jaunder init --skip-if-exists`),
     which applies pending migrations to the existing database. That is what an
     operator's package upgrade does.
3. **Current build: verify.**
   - The current `e2e-support` package's `upgrade.spec.ts` (phase `upgraded`)
     restores the `storageState` and reads `BaselineState`.
   - It runs `verifyProductionBaseline`, which is read-only. That check includes
     the original author session surviving the upgrade.
   - It then runs `verifyFreshAppPasswordLifecycle` to show that new writes work
     on the migrated data.

`upgrade.spec.ts` is a thin wrapper around the existing helpers. It is skipped
unless `JAUNDER_UPGRADE_PHASE` is set, so ordinary lanes are unaffected.
`production-baseline-flow.spec.ts` keeps proving that create and verify agree
within one version. The full current suite is not run on the upgraded site: its
tests provision their own data and never read the earlier records.

**The state is a cross-version contract, bounded by the latest tag.**

- This change adds an explicit `version` to `BaselineState`. The current reader
  also accepts a version-less state, which is what the probe commit writes.
- The reader accepts every state shape written since the latest tag, because tag
  runs span every change since the previous release. Readers for older shapes
  are pruned only after a newer tag's upgrade run is green.
- The same rule covers the seeded-manifest parser, `parseSandboxSeedManifest`,
  because that manifest comes from the earlier build's `seed-sandbox-profile`.
- The writer and reader are the same TypeScript module, so no separate schema is
  needed.
- The `legacy1419` fixture branch goes. It exists only for qualification's
  pinned historical harness.

**Which earlier build.** One workflow, `.github/workflows/upgrade.yml`, has
three triggers:

| Trigger                                 | Upgrades from                                                                                                                                          |
| --------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| tag push (`v*`)                         | the previous `v*` tag merged into `<tag>^`: the release-to-release path operators take                                                                 |
| weekly cron                             | the `headSha` of the newest successful run of this workflow on `main` (`gh run list … --branch main --status success`): early warning between releases |
| `workflow_dispatch` with a `from` input | `from`, for example the last tag before cutting a release                                                                                              |

`dev upgrade-test [--from <rev> | --tag <tag>] <backend>` applies these rules,
so the workflow only passes the trigger through and the rules have unit tests.
When there is no earlier tag or green run, the build upgrades from itself, which
still proves the mechanism. Every from-build after this change carries
`upgrade.spec.ts`, so older revisions need no shim. A tag run reports after the
tag exists. To check a release before tagging, dispatch with `from=<last tag>`.

**Manual initiation.** `dev upgrade-dispatch [--from <rev>] [--ref <branch>]`
starts the CI workflow from a shell, the same way the other `dev` GitHub helpers
call `gh`.

- It runs `gh workflow run upgrade.yml --ref <ref> [-f from=<sha>]`.
- `<rev>` is resolved locally to a full SHA that must exist on `origin`.
- `--ref` defaults to `main`.
- Without `--from`, the workflow applies its own last-green rule.
- It prints the URL of the run it started and does not wait. `gh run watch`
  observes the run.

`dev upgrade-test` remains the local, in-VM equivalent.

**Getting the earlier build in.** This follows the existing `performanceLib`
precedent (`flake.nix`, `tools/dev/dev/perf.py`). The main flake exports
`upgradeLib.mkUpgradeCheck { previous, backend }` and adds no flake inputs.
`dev upgrade-test` resolves the earlier build to a local commit. It builds with
`--impure --expr`, using `builtins.getFlake` on the current checkout and on
`git+file://<checkout>?rev=<sha>`, so any fetched commit works, including
unpushed probe commits.

- The earlier `jaunder`, `test-support` and `e2e-support` come from
  `previous.packages.x86_64-linux`.
- The earlier `nodejs` and Playwright browsers come from
  `previous.inputs.nixpkgs`.
- `flake.lock` stays untouched (ADR-0251).
- CI checks out full history and uploads `.xtask/run/` and
  `.xtask/diagnostics/`, as the e2e jobs do.

**Retirement.**

- **Code and Nix:**
  - delete `tools/qualification` (about 8.3k lines);
  - delete `nix/qualification*.nix` and `nix/system-artifact-qualification.nix`;
  - delete the production-baseline VM module, its configurations, apps and proxy
    (`nix/nixos.nix`), and `lib.productionBaselineVm`;
  - delete the `qualification-host`, `-harness` and `-legacy-1419` packages;
  - delete the `system-artifacts --fixture` path, and the related Nix in
    `nix/packages.nix` and `nix/checks.nix`.
- **`dev`:** delete the `production-baseline-*` commands, `qualification.py` and
  their tests.
- **e2e:** delete `production-baseline-harness.spec.ts`,
  `styling-qualification.ts`, and `JAUNDER_PRODUCTION_BASELINE_TLS` in
  `end2end/playwright.config.ts`.
- **Docs:** delete `docs/production-baseline.md` and its schema.
- **Cargo:**
  - remove the `qualification` features in `host` and `system-artifacts`, and
    the dev-deps of `storage`, `server` and `tools/host-proof`;
  - drop the `tools` workspace member;
  - drop `jsonschema` from `tools/Cargo.lock` (regenerated).
- **Prose:** remove the stray mentions in `common/src/backup.rs`,
  `server/tests/misc/backup_corpus/README.md` and
  `tools/host-proof/src/steps/csr_prerequisite.rs`, and the
  `lint_suppression_check.rs` paths.

**Fixtures.** The deployment fixture builds (`a`/`b-app`/`b-theme`) go. About 57
unit-test call sites in storage, server and host-proof need "an inventory whose
application stylesheet or Studio theme differs". They get it from a small
`test-utils` helper in `host::system_theme` that returns those two variant
inventories. There is no fixture enum, no token parsing and no build-time
selection.

**Decision record.** A `jaunder-adr` draft records the upgrade-test contract:
create, verify, upgrade, verify; state compatibility back to the latest tag; and
the from-build rules (previous tag, last green run on `main`, or an explicit
`from`).

- It supersedes ADR-0239.
- It amends ADR-0261's `qualification` verdict and its consequence.
- It amends ADR-0225, ADR-0247, ADR-0251 and ADR-0258.

`docs/ARCHITECTURE.md`, `docs/application-assets.md`, `CONTRIBUTING.md`,
`README.md` and `docs/README.md` are updated in the same change.

**Accepted losses (beyond #1977's).**

- The earlier build runs under the _current_ NixOS module, so upgrade breakage
  inside the module itself is not covered.
- No browser is retained across the upgrade, and there is no explicit probe that
  the earlier build's stylesheet URLs still resolve. Retained theme revisions
  and immutable headers stay covered by the ordinary tests #1977 listed.
- Cross-backend restore after an upgrade is not covered; ordinary backup-interop
  tests own that. The discover/accept evidence reports and their runbook go.
  #1419 is an operator exercise, and this run is its upgrade evidence.

## Acceptance

- **Proof to execute** (run evidence, not a committed report). The log shows the
  baseline phase passing on earlier binaries only, the schema version unchanged
  until the current `preStart` migrates it, and the upgraded phase passing:
  - `dev upgrade-test --from <sha> <backend>` passes on SQLite and on
    PostgreSQL, with `--from` set to a real `main` commit whose schema is older
    than HEAD's. That commit needs no `upgrade.spec.ts`; see the probe note
    below.
  - `dev upgrade-test --from HEAD` passes on both backends.
- **Probe note.** Commits before this change lack `upgrade.spec.ts`. The
  older-schema proof therefore runs against a throwaway branch: an older `main`
  commit with only the wrapper spec cherry-picked onto it. It is not committed.
- **Ordinary e2e is unaffected.** `upgrade.spec.ts` skips there, and
  `production-baseline-flow.spec.ts` still passes. Ordinary PR CI and
  `dev validate-no-e2e` pass.
- **Manual dispatch works.** Before merge, a unit test covers how arguments
  translate to the `gh` call, and rev resolution, including a rev absent from
  `origin`. GitHub allows dispatch only once `upgrade.yml` is on the default
  branch. The first live `dev upgrade-dispatch` on `main` is therefore a
  post-merge check, reported on the issue.
- **The workflow is complete.** `.github/workflows/upgrade.yml` has the `v*` tag
  trigger and its previous-tag lookup, the weekly cron, a `workflow_dispatch`
  `from` input, `actions: read`, the main-scoped last-green lookup, a
  full-history checkout, a per-backend matrix, and both artifact uploads.
- **Retirement is complete.**
  `rg 'qualification::|features.*qualification|qualification-host|production-baseline-(smoke|discover|accept|qualify|harness|proxy)|productionBaselineVm|legacy1419'`
  finds nothing outside `docs/adr/` and `docs/superpowers/`.
- **Docs land with the code.** The ADR draft and the doc updates are in the same
  change.

## Boundaries

- No golden files, no byte or pixel comparisons, and no new assertion suite.
  Verification is the existing `verifyProductionBaseline`.
- No change to backup or restore semantics, and no new seeding corpus.
- The upgrade test is not added to PR CI, and the full lane catalog is not run
  in upgrade mode.
- No retained-browser or cross-version stylesheet probe.
