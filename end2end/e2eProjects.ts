/**
 * The gated browsers' Playwright project topology, shared by the config and
 * the census contract.
 *
 * Every gated browser has exactly two projects: `<browser>-visual` runs the
 * `@visual` tests with no retries, and `<browser>` runs everything else.
 * Neither has a dependency: Playwright's `--shard` reruns dependency projects
 * in full in every shard, so the lanes (`end2end/e2e-lanes.json`) shard both
 * projects together and each test runs exactly once per backend/browser pair.
 */

export type GatedBrowser = "chromium" | "firefox";
export const gatedBrowsers: readonly GatedBrowser[] = ["chromium", "firefox"];

export function visualProject(browser: GatedBrowser): string {
  return `${browser}-visual`;
}

export function specPattern(specs: readonly string[]): RegExp {
  return new RegExp(`(${specs.join("|")})\\.spec\\.ts`);
}
