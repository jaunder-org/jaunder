import { devices, defineConfig } from "@playwright/test";

const traceParent = process.env.JAUNDER_E2E_TRACEPARENT;
// Worker count is env-driven (#155), default 2: two browser instances per combo
// stays robust on a small/loaded host and lets all four {backend×browser} combos
// run concurrently, while still cutting the serial wall-clock (Firefox was the long
// pole). The host driver (`cargo xtask e2e-local`) overrides this to 1 for its debug
// wasm build. Measured in docs/observability.md #155.
const workers = parseInt(process.env.JAUNDER_E2E_WORKERS || "2", 10);

// Automatic retry of a failed test, env-driven, default 0 (fail-fast — right for
// local `cargo xtask e2e-local` debugging). The CI/`validate` warm gate sets
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

export default defineConfig({
  testDir: "./tests",
  timeout: 30 * 1000,
  expect: { timeout: 5000 },
  fullyParallel: workers > 1,
  forbidOnly: !!process.env.CI,
  workers,
  retries,
  // CI default reporter: streamed line output for the build log + a machine-readable
  // report at the conventional name, inside the default outputDir. The host driver
  // (cargo xtask e2e-local) overrides this with --reporter=html,line for interactive
  // runs (#153).
  reporter: [["line"], ["json", { outputFile: "test-results/results.json" }]],
  use: {
    actionTimeout: 0,
    // Capture forensics only on failure so a green run writes nothing extra (#123/#49).
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
    ...(traceParent ? { extraHTTPHeaders: { traceparent: traceParent } } : {}),
  },
  // admin-site and invite mutate global site-config singletons (site.title/base_url;
  // site.registration_policy, #433), so under fullyParallel they must not overlap specs
  // that read them (ADR-0039). Each browser splits into a parallel main project (these
  // specs ignored) and a serial *-admin project that runs them AFTER the main project
  // (dependencies + fullyParallel:false). At workers=1 this is inert. webkit is defined
  // for host use; the VM never selects it (WPE SIGABRT), so no gating needed.
  projects: [
    {
      name: "chromium",
      testIgnore: /(admin-site|invite)\.spec\.ts/,
      use: {
        ...devices["Desktop Chrome"],
        launchOptions: chromiumLaunchOptions,
      },
    },
    {
      name: "chromium-admin",
      testMatch: /(admin-site|invite)\.spec\.ts/,
      fullyParallel: false,
      dependencies: ["chromium"],
      use: {
        ...devices["Desktop Chrome"],
        launchOptions: chromiumLaunchOptions,
      },
    },
    {
      name: "firefox",
      testIgnore: /(admin-site|invite)\.spec\.ts/,
      use: {
        ...devices["Desktop Firefox"],
        launchOptions: firefoxLaunchOptions,
      },
    },
    {
      name: "firefox-admin",
      testMatch: /(admin-site|invite)\.spec\.ts/,
      fullyParallel: false,
      dependencies: ["firefox"],
      use: {
        ...devices["Desktop Firefox"],
        launchOptions: firefoxLaunchOptions,
      },
    },
    {
      name: "webkit",
      testIgnore: /(admin-site|invite)\.spec\.ts/,
      use: { ...devices["Desktop Safari"] },
    },
  ],
});
