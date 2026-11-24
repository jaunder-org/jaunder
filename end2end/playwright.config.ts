import { devices, defineConfig } from "@playwright/test";
import type { PlaywrightTestConfig } from "@playwright/test";
import { performanceEnabled } from "./tests/browser-performance";
import { parseInstances } from "./tests/instance";
import { type GatedBrowser, gatedBrowsers, specPattern } from "./e2eProjects";

type Project = NonNullable<PlaywrightTestConfig["projects"]>[number];

const traceParent = process.env.JAUNDER_E2E_TRACEPARENT;
// Worker count is env-driven (#155), default 2: two browser instances per combo
// stays robust on a small/loaded host and lets all four {backend×browser} combos
// run concurrently, while still cutting the serial wall-clock. The host driver
// overrides this to 1 for its debug wasm build.
// Rationale: docs/adr/0039-e2e-parallelism-via-per-test-identity-fixtures.md.
const workers = parseInt(process.env.JAUNDER_E2E_WORKERS || "2", 10);
// Each worker binds to its own server instance by parallel index (#2076), so
// the harness's instance map must describe exactly one instance per worker.
const instanceMap = process.env.JAUNDER_E2E_INSTANCES;
const instanceCount =
  instanceMap === undefined ? workers : parseInstances(instanceMap).length;
if (instanceCount !== workers) {
  throw new Error(
    `JAUNDER_E2E_INSTANCES has ${instanceCount} entries but workers is ${workers}`,
  );
}

// Automatic retry of a failed test, env-driven, default 0 (fail-fast — right for
// local debugging). The hermetic e2e checks set
// JAUNDER_E2E_RETRIES=1 (flake.nix), so a test that fails then passes is reported
// `flaky` (exit 0) instead of failing the check: it contains the timeout
// flakiness that otherwise reds an unrelated PR, while the JSON report still
// records the flake for surfacing. Mirrors the JAUNDER_E2E_WORKERS pattern (#155).
const retries = parseInt(process.env.JAUNDER_E2E_RETRIES || "0", 10);

// Firefox in a headless VM defaults to Fission + a content-process pool; each
// Playwright worker is a separate instance, so RSS multiplies. These prefs
// collapse each instance to one content process and trim caches — transparent to
// the app-level tests, and harmless on the host (#155, #61).
const firefoxLaunchOptions = {
  firefoxUserPrefs: {
    "fission.autostart": false,
    "dom.ipc.processCount": 1,
    "dom.ipc.processCount.webIsolated": 1,
    "browser.sessionhistory.max_total_viewers": 0,
    "browser.cache.memory.capacity": 51200,
  },
};

// Applied on every chromium project. Required in the Nix VM (runs as root);
// benign for a throwaway test browser locally, so shared rather than gated (#153).
const chromiumLaunchOptions = {
  args: ["--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage"],
};
const visualTag = /@visual/;
const censusPreflight = Boolean(process.env.JAUNDER_E2E_CENSUS_PREPARE);
const laneRun = Boolean(process.env.JAUNDER_E2E_LANE);
const censusOutput =
  process.env.JAUNDER_E2E_CENSUS_OUTPUT ??
  "test-results/e2e-expected-census.json";
const diagnosticCoverage = Boolean(process.env.JAUNDER_WASM_COVERAGE_OUT);
const measurementMode = Boolean(process.env.JAUNDER_WASM_COVERAGE_MODE);
const performanceMode = performanceEnabled();
const runnerTrace = performanceMode ? "off" : "retain-on-failure";
const diagnosticCoverageSpec = /wasm-coverage\.spec\.ts/;
const measurementSpec = /wasm-coverage-measure\.spec\.ts/;
const performanceMeasurementSpec = /browser-performance\.measure\.spec\.ts/;
// The upgrade check (nix/checks.nix `mkUpgradeCheck`) alone runs this spec.
const upgradeMode = Boolean(process.env.JAUNDER_UPGRADE_PHASE);
const upgradeSpec = /upgrade\.spec\.ts/;
const diagnosticSpec = measurementMode
  ? measurementSpec
  : diagnosticCoverageSpec;
// Specs that only a specialized run selects: wasm coverage and its
// measurement, the browser performance measurement, and the upgrade check.
const unavailableSpecializedSpecs: RegExp[] = [
  ...(diagnosticCoverage ? [] : [diagnosticCoverageSpec, measurementSpec]),
  ...(performanceMode ? [] : [performanceMeasurementSpec]),
  ...(upgradeMode ? [] : [upgradeSpec]),
];

const gatedBrowserUse: Record<GatedBrowser, Project["use"]> = {
  chromium: {
    ...devices["Desktop Chrome"],
    launchOptions: chromiumLaunchOptions,
  },
  firefox: {
    ...devices["Desktop Firefox"],
    launchOptions: firefoxLaunchOptions,
  },
};

/**
 * One gated browser's project (see `e2eProjects.ts`). Every worker owns its own
 * server instance and every test sets the global state it depends on (#2076),
 * so no test needs ordering, serialization or a dependency.
 */
function gatedProject(browser: GatedBrowser): Project {
  const use = gatedBrowserUse[browser];
  return {
    name: browser,
    testIgnore: unavailableSpecializedSpecs,
    ...(diagnosticCoverage ? { testMatch: diagnosticSpec } : {}),
    // The reviewed baselines keep the names of the retired `<browser>-visual`
    // project.
    snapshotPathTemplate: `{snapshotDir}/{testFilePath}-snapshots/{arg}-${browser}-visual-{platform}{ext}`,
    use,
  };
}

export default defineConfig({
  testDir: "./tests",
  timeout: 30 * 1000,
  // 10s assertion timeout (not Playwright's 2s / our old 5s): this CSR app populates
  // fields from async config/data fetches, which on the slowest combo (firefox+sqlite)
  // can exceed 5s — the #609 admin-site base_url flake. 10s is the value assertions
  // across the suite already bumped to by hand, so making it the default lets those
  // per-assertion overrides go. A genuine assertion failure now surfaces in 10s,
  // bounded by the whole-test budget (30s base above, scaled per-test for slow
  // browsers by the `auto` fixture via slowBrowserTimeoutMs).
  expect: { timeout: 10000 },
  fullyParallel: workers > 1,
  forbidOnly: !!process.env.CI,
  workers,
  retries,
  // CI default reporter: one `list` line per finished attempt (status, project,
  // file:line, title, retry, duration) for the build log, which survives a
  // killed run (#2011), + a machine-readable report at the conventional name,
  // inside the default outputDir. The host driver overrides this with
  // --reporter=html,line for interactive runs (#153).
  reporter: [
    ["list"],
    ["json", { outputFile: "test-results/results.json" }],
    ...(censusPreflight
      ? [["./e2eCensus.ts", { outputFile: censusOutput }] as const]
      : laneRun
        ? [["./e2eLaneManifest.ts"] as const]
        : []),
    [
      "./durationBudgetManifest.ts",
      { outputFile: "test-results/duration-budget-manifest.json" },
    ],
  ],
  use: {
    actionTimeout: 0,
    // Performance measurements own one trace per sample; all other runs retain
    // failure forensics without writing artifacts for green tests (#123/#49).
    trace: runnerTrace,
    screenshot: "only-on-failure",
    ...(traceParent ? { extraHTTPHeaders: { traceparent: traceParent } } : {}),
  },
  // Each gated browser's one project, then WebKit, which runs only host-local
  // investigation. WebKit never covered visual, admin-site, invite or the
  // production baseline.
  projects: [
    ...gatedBrowsers.map(gatedProject),
    ...(upgradeMode
      ? [
          {
            name: "chromium-upgrade",
            testMatch: upgradeSpec,
            fullyParallel: false,
            workers: 1,
            use: gatedBrowserUse.chromium,
          },
        ]
      : []),
    {
      name: "webkit",
      testIgnore: [
        ...unavailableSpecializedSpecs,
        specPattern(["admin-site", "invite", "production-baseline-flow"]),
      ],
      grepInvert: visualTag,
      use: { ...devices["Desktop Safari"] },
    },
  ],
});
