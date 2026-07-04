# Spec — Issue #153: one source of truth for the Playwright e2e config

- **Issue:** jaunder-org/jaunder#153 (`dx`, milestone "E2E test suite")
- **Branch/worktree:** `worktree-issue-153-dedupe-playwright-config`
- **Status:** design resolved; awaiting spec approval.

## Problem

Two Playwright configs drive the e2e suite and have diverged:

- `end2end/playwright.config.ts` — the **host** config. Loaded by
  `end2end/run-e2e.sh` (the leptos `end2end-cmd`, `Cargo.toml:132`), which
  hardcodes `playwright test --project chromium --workers=1`.
- `nixPlaywrightConfig` — an inline `writeText` JS string in `flake.nix:509`. It
  is the config that actually runs in CI and `cargo xtask validate` /
  `cargo xtask e2e`, via the NixOS VM
  (`--config playwright.nix.config.js --project ${browser} --project ${browser}-admin`).

They drift across reporter, retries, workers, per-project timeouts, project set,
Firefox process-slimming, chromium launch args, trace/screenshot policy, and
outputDir (see the issue table + the #155 comment). Result: "passes locally" ≠
"passes in CI", which bit the #152 investigation. A secondary problem surfaced
during design: the host path is **currently unused** — no jaunder skill tells
contributors to run it — so today it is drift-prone dead weight rather than a
tool.

## Design decision (resolved)

**One shared config file consumed by both runners, with the only permitted
differences being an explicit, enumerated set of environment-forced values
injected via env vars / env-gated branches.** This is the only structure where
divergence is _structurally_ confined: a shared object that both sides
spread-and-override, or a factory with a per-environment wrapper, both leave a
second artifact (or open override site) that can silently re-diverge —
reintroducing exactly this issue. A single file cannot.

Consequently:

1. **`end2end/playwright.config.ts` becomes the single source of truth.** All
   shared settings live there once.
2. **`nixPlaywrightConfig` (the `writeText` in `flake.nix`) is deleted.** The VM
   `testScript` loads the real file: `--config playwright.config.ts`. The whole
   `end2end/` tree is already staged into the VM (`e2ePackage`,
   `flake.nix:647`), and Playwright already loads `.ts` there (the test files
   are `.ts`), so this needs no new staging.
3. **The shared config is byte-identical for both runners; the only host/VM
   difference is an invocation flag in the host driver, not config drift** — and
   it needs no custom env vars:
   - **reporter** — the shared config defaults to
     `[['line'], ['json', { outputFile: 'test-results/results.json' }]]` (what
     CI needs: streamed line output for the build log + a machine-readable
     report). The **host driver overrides it at invocation** with Playwright's
     `--reporter=html,line` CLI flag (which replaces the config's reporter
     list), so interactive local runs get the HTML report. The reporter
     difference thus lives visibly in the _runner_, not a hidden config branch,
     and the host — having replaced the reporter — emits no json file (nothing
     to gitignore). **The driver must set `PLAYWRIGHT_HTML_OPEN=never`**: the
     HTML reporter defaults to `open: 'on-failure'`, which on a _red_ local run
     would spawn a blocking `playwright show-report` server + browser — hanging
     exactly the iterate-on-failure case the fast loop exists for. (Since the
     reporter is CLI-set, the env var is the way to pin `open`.)
   - **json report path needs no env var, and uses conventional locations.**
     `outputFile` is the _relative_ path `test-results/results.json` —
     `results.json` is the filename Playwright's own docs use (the reporter has
     no default file; its default is _stdout_), and it sits inside
     `test-results/` (the default `outputDir`) so it groups with the other e2e
     outputs and is explicable to any Playwright user. CWD and config-dir
     coincide in each environment (`/tmp/e2e` in the VM per `flake.nix:727`;
     `end2end/` on host), so it resolves to `/tmp/e2e/test-results/results.json`
     in the VM. This _does_ change the `e2e-gate` flat-copy source path
     (`flake.nix:765`) by one segment — a justified edit — **but the copied-out
     artifact name `playwright-report-${backend}.json` is preserved unchanged**,
     because it is a stable external contract consumed by
     `docs/observability.md`, the xtask rescue predicate
     (`name.starts_with("playwright-report-")`), and the #152 trace-analysis
     tooling. The report now also rides the `tar -C /tmp/e2e test-results`
     bundle (`768`) automatically — harmless, and arguably nicer (it travels
     with the traces it references). (Implementer note: Playwright cleans
     `outputDir` once, _before_ the first test; the JSON reporter writes in
     `onEnd`, _after_ all tests — so `results.json` living inside
     `test-results/` is not wiped.)
   - **chromium `--no-sandbox --disable-gpu --disable-dev-shm-usage` are applied
     always**, as a shared `launchOptions.args`. Required in the VM (runs as
     root) and benign for a throwaway test browser locally, so sharing them
     removes the delta rather than gating it. _(The one intentional change to
     host chromium launch behavior; the fallback is to gate them, which
     reintroduces the env plumbing this avoids.)_
   - **`outputDir` uses the default**, which (config-dir-relative) resolves to
     `/tmp/e2e/test-results` in the VM — where `tar -C /tmp/e2e test-results`
     (`flake.nix:768`) expects it — and to a host-local dir otherwise. No
     injection.

   The net flake change is therefore small and env-plumbing-free: delete the
   `nixPlaywrightConfig` `writeText`, drop the two `cp ${nixPlaywrightConfig} …`
   lines (`flake.nix:866`, `970`), point the run at
   `--config playwright.config.ts`, and **scrub the remaining doc-comment
   references** to `nixPlaywrightConfig` (e.g. `flake.nix:1052`, and the
   `workers` note) so the name is genuinely gone (AC1).

4. **Settings that only _looked_ environment-specific become identical shared
   settings**:
   - **webkit** needs no gating — the VM selects `--project chromium/firefox`,
     so a webkit project defined in the shared file is simply never launched in
     the VM (no WPE SIGABRT). It stays available for host runs for free.
   - **admin-site quarantine** (`*-admin` projects, `dependencies` +
     `fullyParallel:false`), **Firefox process-slimming** (`firefoxUserPrefs`),
     **workers** (`JAUNDER_E2E_WORKERS`), **retries**, and **timeout** all
     become one shared value. Applying the quarantine + slimming on the host too
     is a correctness improvement (ADR-0039), not a regression.
   - **trace/screenshot policy is pinned to the VM's failure-capture values**:
     `trace: "retain-on-failure"` + `screenshot: "only-on-failure"`, _not_ the
     host's current `trace: "on-first-retry"`. This is load-bearing: the VM runs
     with `retries: 0`, so an `on-first-retry` policy would capture _nothing_ on
     failure and silently defeat AC6. The shared value must be the
     retain-on-failure form.
   - The host's duplicated `hydrationHeavyTimeoutScale = 2.2` constant
     (`playwright.config.ts:6`) and static per-project timeouts are removed;
     per-test budgets come from the already-centralized `fixtures.ts` helpers,
     which self-adapt to the resolved worker count (#155).

5. **`end2end/run-e2e.sh` is promoted to a `cargo xtask e2e-local` driver** that
   seeds fixtures and runs Playwright against the host dev server, making the
   host loop a first-class xtask verb consistent with the rest of the tooling.
   Concretely it must:
   - select **`--project chromium --project chromium-admin`** (mirroring the
     VM), _not_ `--project chromium` alone — under the unified config the
     `chromium` project carries `testIgnore: /admin-site\.spec\.ts/`, so
     selecting only `chromium` would silently drop admin-site coverage that
     today's flat host config runs (a regression);
   - pass `--reporter=html,line` and set `PLAYWRIGHT_HTML_OPEN=never` (Design
     §3);
   - **own the server lifecycle explicitly.** Today `run-e2e.sh` runs _inside_ a
     server that cargo-leptos has already started (`end2end-cmd`,
     `Cargo.toml:132`); a standalone `cargo xtask e2e-local` must either
     start/stop its own dev server or document that it assumes a running one.
     The plan must pick one and name it — this is not left implicit.

   `run-e2e.sh` is removed once the driver replaces it, and `Cargo.toml:132`'s
   `end2end-cmd` is updated to the new driver (or the leptos hook retired in its
   favor — the plan decides, consistent with the server-lifecycle choice above).

6. **jaunder skill guidance is added telling contributors to use the host fast
   loop**, including the single-test invocation, so the retained path is
   documented and used rather than dead.

### What this trade gives up (named, accepted)

Casual/invisible divergence is gone by construction. Any _intentional_ future
host-vs-CI difference must be added as an explicit env-gated branch in the one
file, visible in review. That friction is the point.

## Risks & required verification

Two failure modes, asymmetric in nastiness:

- **VM `.ts` config load (loud, low risk).** The unified file uses `import` /
  `export default` where `nixPlaywrightConfig` was CJS. This is _not_ a Node
  ESM-resolver concern: `end2end/package.json` has no `"type"` field (defaults
  to CommonJS), yet the existing `playwright.config.ts` and every `.spec.ts`
  already use `import` and load fine, because Playwright transpiles its `.ts`
  config with its own bundler regardless of package `type`. So the load path is
  Playwright's transform, already exercised in the VM. Risk is genuinely low and
  fails _loudly_ (config won't load) if wrong — AC5 pins it.
- **json report path (quiet, now low risk).** The json report is written to the
  relative `test-results/results.json`, resolving to
  `/tmp/e2e/test-results/results.json` in the VM; the `e2e-gate` flat-copy
  (`flake.nix:765`) is repointed there but keeps emitting the contract-stable
  `playwright-report-${backend}.json`. Because CWD and config-dir coincide at
  `/tmp/e2e` in the VM, the relative path is robust regardless of Playwright's
  resolution base — the earlier env-injection framing was over-engineering.
  `outputDir` is likewise safe on its default (see Design §3). Still
  belt-and-suspenders guarded by the **red-run** AC6, since a missing report
  only surfaces when a CI test fails.

## Acceptance criteria

Each is stated so ship-time conformance review can tell delivered from not.

- **AC1 — Single source.** The token `nixPlaywrightConfig` no longer appears
  anywhere in `flake.nix` (not just the `writeText` — the doc-comment references
  at `flake.nix:1052` and the `workers` note are scrubbed too); the VM
  `testScript` runs `playwright test --config playwright.config.ts …`.
  `end2end/` contains exactly one Playwright config file.
- **AC2 — No silent behavioral drift.** The host and VM load the _same_ config
  file, so every setting (retries, workers default, timeout, trace/screenshot
  policy, admin-site quarantine, Firefox slimming, chromium launch args, project
  definitions) is identical by construction. The only host/VM difference is an
  explicit invocation flag — the host driver's `--reporter=html,line` override —
  visible in the runner, not the config. (Checkable: `flake.nix` references no
  `playwright.nix.config.js` and sets no Playwright-reporter env var for the run
  — the run passes `--config playwright.config.ts` and nothing further.)
- **AC3 — Host loop is a first-class xtask verb.** `cargo xtask e2e-local` runs
  the host chromium e2e (seed +
  `playwright test --project chromium --project chromium-admin`) against the dev
  server, with server-lifecycle ownership resolved per Design §5.
  `end2end/run-e2e.sh` is removed and `Cargo.toml`'s e2e wiring points at the
  new driver.
- **AC4 — Host loop is documented and directed.** jaunder skill guidance
  instructs contributors to use the host fast loop, including how to run a
  single e2e test. (Grep for `cargo xtask e2e-local` — the concrete command name
  — in `.claude/skills/` / `docs/agents/` returns the guidance.)
- **AC5 — VM loads the unified config.** `cargo xtask e2e sqlite chromium` runs
  green on the branch with the unified config (proves risk #1 dead).
- **AC6 — Red-run diagnostics survive.** With a deliberately-failed test in a VM
  combo (`cargo xtask e2e <backend> <browser>`), the failure recovers a
  diagnostics bundle containing Playwright traces + screenshots and the json
  report. Because a red run fails the nixosTest check, the evidence lands
  **not** in `$out` but in the `--keep-failed` build dir, recovered by xtask's
  `rescue_diagnostics` to `.xtask/diagnostics/e2e-<backend>-<browser>/`
  (`flake.nix:711`, `xtask/src/steps/nix.rs:298`). Conformance check: that
  directory contains `playwright-artifacts-<backend>.tar.gz` (the tarred
  `test-results/` trace archive + screenshots) and
  `playwright-report-<backend>.json` after a failed run. (Evidence is a red run,
  not a green one.)
- **AC7 — Full-matrix parity.** `cargo xtask validate` (all four
  `{sqlite,postgres}×{chromium,firefox}` combos) is green on the branch,
  confirming Firefox slimming still applies and the admin quarantine is still
  serial across every combo.
- **AC8 — Host loop parity + new capability.** The new host driver runs
  `chromium` + `chromium-admin` green against the host server (HTML report
  produced, and a _red_ run does **not** hang on an auto-opened report server —
  `PLAYWRIGHT_HTML_OPEN=never` per Design §3/§5); a `workers=2` host run
  confirms the admin-site quarantine engages locally — observable via
  `chromium-admin` running serially _after_ `chromium` (its
  `dependencies: ['chromium']` + `fullyParallel: false`), i.e. admin-site never
  overlapping the parallel specs. This is the capability the fast loop is
  retained for, so the run must exercise both projects.

## Out of scope / separable

- Any change to the per-test budget helpers in `fixtures.ts` (already
  config-source-agnostic post-#155). This spec only _removes_ the host config's
  duplicate constant; it does not touch the helper logic.
- Changing the CI e2e matrix shape or the `e2e-gate` aggregation (ADR-0034) —
  unchanged.

## Relations

- Surfaced during #152 (Firefox e2e slowdown). Divergence grew in #155/PR #225.
- Touches ADR-0039 (admin-site singleton serialization), ADR-0034 (CI e2e
  matrix).
