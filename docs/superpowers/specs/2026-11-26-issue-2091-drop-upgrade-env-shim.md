# Issue #2091 — drop the upgrade check's pre-instance-map env shim

## Outcome

The upgrade check's first phase (`e2eUpgradeBaseline` in `nix/checks.nix`)
passes the earlier build's Playwright run only the instance map
(`JAUNDER_E2E_WORKERS` + `JAUNDER_E2E_INSTANCES`) plus its upgrade-phase
variables. The four legacy variables (`JAUNDER_E2E_BASE_URL`,
`JAUNDER_CAPTURE_DIR`, `JAUNDER_DB`, `JAUNDER_STORAGE_PATH`) and their comment
are deleted.

## Why now (the issue's trigger is met)

- **Default start point.** `dev upgrade-test` without `--from`/`--tag` starts
  from the newest green `upgrade.yml` run on `main`: today `0d705563f`, which
  contains #2076 (`f2dc27273`). Its `end2end/tests/instance.ts` sets `BASE_URL`,
  `JAUNDER_DB`, `JAUNDER_STORAGE_PATH` and `JAUNDER_CAPTURE_DIR` in each worker
  from the map, and its `playwright.config.ts` reads no legacy variable in the
  runner process. Every later green run also contains #2076.
- **`--tag`.** The repository has no `v*` tags. A tag run with no earlier tag
  upgrades from the tag itself (`tools/dev/dev/upgrade.py` `earlier`), so the
  first release tag necessarily post-dates #2076.

## Approach

Delete lines 626-632 of `nix/checks.nix`. No other code, doc or ADR names the
shim; ADR-0269 already states the map replaces `JAUNDER_E2E_BASE_URL`.

## Acceptance

Proof to execute (not committed):

- `dev upgrade-test sqlite` and `dev upgrade-test postgres` are green from the
  default earlier build.
- `dev upgrade-test sqlite --tag HEAD` and `... postgres --tag HEAD` are green.
  With no `v*` tag this resolves to upgrading from the checkout itself, which is
  exactly what a first release-tag run does. It satisfies the issue's `--tag`
  criterion but never exercised the shim; the default run (from `0d705563f`) is
  the one that shows the deletion is safe.
- `dev check` passes on the change.

Deliverable: the deleted block in `nix/checks.nix`.

## Boundaries

- No change to `upgrade.py`'s start-point selection, the upgrade spec, or the
  e2e instance-map harness.
- No release tag is created.
- Accepted cost: an explicit `--from <sha>` (or the workflow's `from=` input)
  naming one of the few `main` commits that have the upgrade spec but predate
  #2076 (landed 2026-11-24, 19:55Z to 22:47Z) no longer works as a start point.
  Neither the default nor `--tag` can select one.
