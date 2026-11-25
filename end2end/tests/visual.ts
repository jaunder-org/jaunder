import { join } from "node:path";

import { expect, type Locator, type Page } from "@playwright/test";
import { test, type NewTracedContext } from "./fixtures";
import {
  applySeededSession,
  createSessionViaTool,
  seedConfigViaTool,
  seedUserViaTool,
  type SeedRecord,
} from "./seed";
import { selectSiteTheme } from "./theme-helpers";
import { runVisualAssertion } from "./visual-diagnostics";

const stylePath = join(__dirname, "visual.css");

type VisualOptions = {
  mask?: Locator[];
  stylePath?: string;
};

/**
 * Set the seeded site configuration that the visual baselines render: the
 * default title with no tagline, the seeded base URL (it decides whether
 * passkeys are available, which `/login` shows), Open registration, the Private
 * Site Default Audience, an enabled Media Upload Capability and no Site Theme
 * selection. Other tests change these globals on the same instance.
 */
export async function setSeededSiteAppearance(
  tracedContext: NewTracedContext,
): Promise<void> {
  await seedConfigViaTool("site.title", "Jaunder");
  await seedConfigViaTool("site.base_url", "https://example.com");
  await seedConfigViaTool("site.tagline", "");
  await seedConfigViaTool("site.registration_policy", "open");
  await seedConfigViaTool("posts.default_audience", "private");
  await seedConfigViaTool("media.uploads_enabled", "true");
  await selectSiteTheme(tracedContext, null);
}

/**
 * Sign `page` in as a visual test's fixed-name user, seeding the user when it
 * does not exist yet. The name renders in the screenshot, so it cannot vary per
 * attempt, and a retry can land on the instance where an earlier attempt already
 * seeded it; that retry signs in to the existing user instead. Returns whether
 * this call seeded the user, so the caller seeds the user's other state once.
 */
export async function signInAsVisualUser(
  page: Page,
  username: string,
): Promise<boolean> {
  let session: SeedRecord;
  let seeded = true;
  try {
    session = await seedUserViaTool(username, "visualpassword123");
  } catch (seedError) {
    // Only a retry may find its own fixed-name user already seeded.
    if (test.info().retry === 0 || !usernameTaken(seedError)) throw seedError;
    session = await createSessionViaTool(username);
    seeded = false;
  }
  await applySeededSession(page.context(), session);
  return seeded;
}

/** Whether `seed-user` failed only because the username already exists. */
function usernameTaken(error: unknown): boolean {
  const stderr = (error as { stderr?: unknown }).stderr;
  return String(stderr).includes("username is already taken");
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
