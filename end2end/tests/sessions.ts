import type { Page } from "@playwright/test";

import { click, goto } from "./helpers";

/** Mint an App Password through the browser Sessions surface and return it once. */
export async function mintAppPassword(
  page: Page,
  label: string,
): Promise<string> {
  if (new URL(page.url()).pathname !== "/sessions") {
    await goto(page, "/sessions");
  }
  await page.fill("#app-password-label", label);
  await click(page, '.j-app-passwords button:has-text("Create app password")');
  const token = page.locator(".j-app-password-token code");
  await token.waitFor({ state: "visible", timeout: 15_000 });
  return ((await token.textContent()) ?? "").trim();
}
