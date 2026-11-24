# Port the theme release-version test to Python (#2070)

## Outcome

The release behavior of `.github/workflows/theme-repository.yml` (SemVer tag
validation and the "Create immutable release" step) is checked by a pytest
module that finds its steps by YAML structure, so reformatting the workflow
cannot silently break the check.
`scripts/test-theme-repository-release-version.sh` is deleted, the last bash
script that ADR-0261 says does not conform.

## Approach and tradeoffs

- **Where it lives.** A new `tools/dev/tests/test_theme_repository_release.py`,
  following the `test_ci_storage_probe.py` precedent for testing repository
  files from the `dev` suite. Host `pytest tools/dev` already runs in
  `dev check`, `dev prepush` and CI's `validate-host` on every PR. The hermetic
  `dev-python` Nix check's filtered source has no `.github/`, so the module
  skips there through the existing `conftest.require` gate, like the
  storage-probe test.
- **Reading the workflow.** PyYAML loads both workflows; steps are found by job
  and `name:`. PyYAML parses the bare `on:` key as boolean `True`; the test
  reads the fixture's trigger through that key. `pyyaml` and its `types-pyyaml`
  stubs (for `mypy --strict`) join `devPythonCheck` only, the test-tool
  environment; the `dev` runtime gains no dependency.
- **Tag validation runs the real step.** Instead of extracting the regex and
  re-evaluating it, each tag runs the whole "Validate release version" `run:`
  block under `bash` with `TAG` set and asserts exit 0 / nonzero. Same accepted
  and rejected tag lists as today. This tests what CI runs and keeps bash's ERE
  semantics (Python `re` differs).
- **Release step.** The "Create immutable release" `run:` block runs under
  `bash` with a mock `gh` on `PATH`. The mock is a small executable that logs
  its argv and answers `release view` from environment variables, as today. All
  seven existing-release and four absent-release cases keep their expected exit
  status, `release edit` line, upload/download presence and exact
  `release create` line, and each absent-release case still makes exactly one
  `gh` call; all as parametrized tests. Workflows are read inside a fixture, not
  at collection, so the `require` skip works.
- **Fixture-trigger check kept:** the fixture workflow's `on.push.tags` must
  contain `v0.0.0-theme-repository-fixture.[1-9]*`.
- **CI caller.** Remove the fixture workflow's `release-version-contract` job,
  its `needs:` edges and its `paths:` entry for the deleted script. The test now
  runs in `validate-host` on every PR, merge group and push to main, a superset
  of the fixture's path-filtered PRs. Accepted gap: a fixture tag pushed onto a
  commit that never went through CI would release without the check (no fixture
  tag exists today; tags are assumed to be cut from CI'd commits). The
  alternative, keeping the job to run pytest, needs a Nix shell in that job to
  repeat a check CI already ran.

## Acceptance

- The new module passes under
  `traced-run -- pytest tools/dev/tests/test_theme_repository_release.py` and
  fails when a case's expectation is flipped (spot-check one per family).
- No indentation- or line-based matching of workflow text; steps are located
  only by job and step `name:` (review property, plus a one-off manual check:
  reindent `theme-repository.yml` locally, the module still passes).
- `dev-python` Nix check still passes (module skips there); `dev check` green.
- Script deleted; no remaining reference outside `docs/`
  (`rg --hidden test-theme-repository-release-version -g '!docs/**'`).
- `theme-repository-fixture.yml` has no `release-version-contract` job and still
  parses (`package`/`release` without `needs`).

## Boundaries

No change to `theme-repository.yml` behavior or text. No new `scripts/` entry.
ADR-0261 text stays as written (accepted record).
