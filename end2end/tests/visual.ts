import { join } from "node:path";

import { expect, type Locator, type Page } from "@playwright/test";

const stylePath = join(__dirname, "visual.css");

type VisualOptions = {
  mask?: Locator[];
};

/** Compare the current desktop viewport under the deterministic screenshot-only
 * typography contract. Callers own readiness and the one permitted dynamic mask. */
export async function expectVisual(
  page: Page,
  name: string,
  options: VisualOptions = {},
): Promise<void> {
  await page.evaluate(() => document.fonts.ready);
  await expect(page).toHaveScreenshot(name, {
    animations: "disabled",
    caret: "hide",
    mask: options.mask,
    maxDiffPixels: 0,
    stylePath,
    threshold: 0,
  });
}
