import { join } from "node:path";

import { expect, type Locator, type Page } from "@playwright/test";
import type { NewTracedContext } from "./fixtures";
import { seedConfigViaTool } from "./seed";
import { selectSiteTheme } from "./theme-helpers";
import { runVisualAssertion } from "./visual-diagnostics";

const stylePath = join(__dirname, "visual.css");

type VisualOptions = {
  mask?: Locator[];
  stylePath?: string;
};

/**
 * Set the seeded site configuration that the visual baselines render: the
 * default title with no tagline, Open registration, the Private Site Default
 * Audience, enabled Media uploads and no Site Theme selection. Other tests
 * change these globals on the same instance.
 */
export async function setSeededSiteAppearance(
  tracedContext: NewTracedContext,
): Promise<void> {
  await seedConfigViaTool("site.title", "Jaunder");
  await seedConfigViaTool("site.tagline", "");
  await seedConfigViaTool("site.registration_policy", "open");
  await seedConfigViaTool("posts.default_audience", "private");
  await seedConfigViaTool("media.uploads_enabled", "true");
  await selectSiteTheme(tracedContext, null);
}

/** Compare the current desktop viewport under the deterministic screenshot-only
 * typography contract. Callers own readiness and the one permitted dynamic mask. */
export async function expectVisual(
  page: Page,
  name: string,
  options: VisualOptions = {},
): Promise<void> {
  await page.evaluate(() => document.fonts.ready);
  await runVisualAssertion(page, name, () =>
    expect(page).toHaveScreenshot(name, {
      animations: "disabled",
      caret: "hide",
      mask: options.mask,
      maxDiffPixels: 0,
      stylePath: options.stylePath ? [stylePath, options.stylePath] : stylePath,
      threshold: 0,
    }),
  );
}

/** Compare one stable UI region without pinning unrelated page chrome. */
export async function expectVisualRegion(
  page: Page,
  region: Locator,
  name: string,
  options: VisualOptions = {},
): Promise<void> {
  await page.evaluate(() => document.fonts.ready);
  await runVisualAssertion(page, name, () =>
    expect(region).toHaveScreenshot(name, {
      animations: "disabled",
      caret: "hide",
      mask: options.mask,
      maxDiffPixels: 0,
      stylePath: options.stylePath ? [stylePath, options.stylePath] : stylePath,
      threshold: 0,
    }),
  );
}
