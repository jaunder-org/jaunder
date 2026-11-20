/**
 * The gated browsers' Playwright project topology, shared by the config, the
 * census contract and the tests that act only in one project role.
 *
 * Every gated browser has the same four roles, run in this order: visual
 * contracts, parallel ordinary tests, the serial site-settings group (specs
 * that mutate or must restore global site configuration), then invite. The
 * unsplit graph chains them in one Playwright process; the split lanes
 * (`end2end/e2e-lanes.json`) select the `split` projects across isolated VMs.
 */

export type GatedBrowser = "chromium" | "firefox";
export const gatedBrowsers: readonly GatedBrowser[] = ["chromium", "firefox"];

export type Role = "visual" | "ordinary" | "siteSettings" | "invite";
export const roles: readonly Role[] = [
  "visual",
  "ordinary",
  "siteSettings",
  "invite",
];
export type Topology = "unsplit" | "split";

export function projectName(
  browser: GatedBrowser,
  topology: Topology,
  role: Role,
): string {
  if (topology === "unsplit") {
    return {
      visual: `${browser}-visual`,
      ordinary: browser,
      siteSettings: `${browser}-admin-site`,
      invite: `${browser}-admin`,
    }[role];
  }
  return {
    visual: `${browser}-special-visual`,
    ordinary: `${browser}-ordinary`,
    siteSettings: `${browser}-special-global-configuration`,
    invite: `${browser}-special-invite`,
  }[role];
}

/** Every gated project in `role`, across browsers and topologies. */
export function projectsInRole(
  role: Role,
  browsers: readonly GatedBrowser[] = gatedBrowsers,
): string[] {
  return browsers.flatMap((browser) =>
    (["unsplit", "split"] as const).map((topology) =>
      projectName(browser, topology, role),
    ),
  );
}

/**
 * Specs that mutate or must restore global site configuration. Passkey
 * ceremonies need Chromium's virtual authenticator, so only Chromium owns them.
 */
export function siteSettingsSpecs(browser: GatedBrowser): string[] {
  return [
    "admin-site",
    "backup",
    "smtp",
    "theme",
    "theme-management",
    "theme-presentation",
    "responsive-shell",
    "post-image-fit",
    "media",
    ...(browser === "chromium" ? ["passkeys"] : []),
    "timeline-cls",
    "production-baseline-flow",
  ];
}

export const inviteSpecs = ["invite"];

/** Specs whose `@visual` tests are not run by the visual role. */
export const visualIgnoredSpecs = [
  "admin-site",
  "backup",
  "smtp",
  "invite",
  "media",
  "production-baseline-flow",
];

export function specPattern(specs: readonly string[]): RegExp {
  return new RegExp(`(${specs.join("|")})\\.spec\\.ts`);
}
