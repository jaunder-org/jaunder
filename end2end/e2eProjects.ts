/**
 * The gated browsers' Playwright project topology, shared by the config and
 * the census contract.
 *
 * Every gated browser has exactly one project, named for the browser, running
 * every test, `@visual` included. It has no dependency: Playwright's `--shard`
 * reruns dependency projects in full in every shard, so the lanes
 * (`end2end/e2e-lanes.json`) shard the one project and each test runs exactly
 * once per backend/browser pair.
 */

export type GatedBrowser = "chromium" | "firefox";
export const gatedBrowsers: readonly GatedBrowser[] = ["chromium", "firefox"];

export function specPattern(specs: readonly string[]): RegExp {
  return new RegExp(`(${specs.join("|")})\\.spec\\.ts`);
}
