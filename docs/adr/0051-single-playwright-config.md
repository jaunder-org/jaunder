# ADR-0051: One Playwright config for host and CI

- Status: proposed
- Date: 2026-07-04
- Issue: [#153](https://github.com/jaunder-org/jaunder/issues/153)

## Context

The e2e suite was driven by **two** Playwright configs that had diverged: the
host `end2end/playwright.config.ts` (run by `run-e2e.sh`) and an inline
`nixPlaywrightConfig` `writeText` in `flake.nix` (the config CI and
`cargo xtask validate` actually run). They drifted across reporter, retries,
workers, timeouts, project set, Firefox slimming, chromium launch args,
trace/screenshot policy, and outputDir — so "passes locally" and "passes in CI"
were not equivalent, which bit the #152 investigation and grew in #155.

Two configs is inherently drift-prone: any structure where the same settings
live in two places (or a shared base that each side spreads-and-overrides)
leaves a second artifact that can silently re-diverge.

## Decision

**`end2end/playwright.config.ts` is the single source of truth, loaded verbatim
by both runners.** `nixPlaywrightConfig` is deleted; the Nix VM `testScript`
runs `playwright test --config playwright.config.ts` (the whole `end2end/` tree
is already staged into the VM).

The **only** host/VM differences are **invocation flags set by the host
driver**, not branches inside the config:

- reporter — the config defaults to `[['line'], ['json', …]]` (CI needs a
  machine-readable report); the host driver overrides with
  `--reporter=html,line` for interactive use.
- `PLAYWRIGHT_HTML_OPEN=never` — so a red host run doesn't spawn a blocking
  report server.
- `JAUNDER_E2E_WORKERS=1` on the host — the host `cargo leptos end-to-end`
  serves a _debug_ CSR build whose hydration wants full CPU per test; the CI VM
  keeps the config default of 2 (release wasm).

Everything else — admin-site quarantine (ADR-0039), Firefox process-slimming,
chromium `--no-sandbox` launch args (applied always; benign on the host),
retries, per-test timeout budgets, trace/screenshot policy — is one shared
value. The json report is written to `test-results/results.json` (inside the
default `outputDir`); the copied-out CI artifact keeps its stable
`playwright-report-<backend>.json` name.

The host e2e loop is promoted from `end2end/run-e2e.sh` to
**`cargo xtask e2e-local`**, a typed verb that seeds fixtures and runs
Playwright against an already-running dev server. cargo-leptos owns the server
lifecycle (it invokes the driver as `end2end-cmd`); a standalone
`cargo xtask e2e-local` assumes a server is already serving (fast re-runs).

## Consequences

- Host/CI Playwright drift is **structurally impossible** — there is one config
  file. Any intentional future host/VM difference must be added as an explicit
  invocation flag in the driver, visible in review, rather than an invisible
  edit to one of two configs.
- `run-e2e.sh` is retired; the host loop is a first-class `cargo xtask` verb
  consistent with the rest of the tooling.
- Relates to ADR-0039 (admin-site serialization) and ADR-0034 (CI e2e matrix),
  both preserved. Applying the admin quarantine + Firefox slimming on the host
  too is a correctness improvement, not a regression.
- Surfaced follow-ups: #234 (the host CSR build must emit a wasm filename the
  bootstrap loads, or the host loop can't hydrate) and #229
  (`provision-node-modules.sh` → a `devtool` subcommand).
