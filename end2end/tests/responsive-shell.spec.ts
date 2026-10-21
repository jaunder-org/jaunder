import { test, expect } from "./fixtures";
import {
  goto,
  signInAs,
  confirmedMutation,
  type MutationOutcome,
} from "./helpers";
import { openThemeAdvanced, withSiteThemeSelection } from "./theme-helpers";

const THEME_ENDPOINTS = {
  save_editor: "/api/themes/save_editor",
  publish: "/api/themes/publish",
  select: "/api/themes/select",
} as const;

test("public shell gives the main region the narrow viewport width", async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await goto(page, "/");

  const sidebar = await page.locator(".j-sidebar").boundingBox();
  const main = await page.locator(".j-main-region").boundingBox();
  expect(sidebar).not.toBeNull();
  expect(main).not.toBeNull();
  expect(main!.y).toBeGreaterThanOrEqual(sidebar!.y + sidebar!.height);
  expect(main!.width).toBeGreaterThanOrEqual(374);
  expect(
    await page.evaluate(() => document.documentElement.scrollWidth),
  ).toBeLessThanOrEqual(390);
});

test("custom Theme surface reaches the public rail and scroll canvas", async ({
  page,
  tracedContext,
}) => {
  await withSiteThemeSelection(tracedContext, async (mutate) => {
    await signInAs(page, "testoperator");
    await goto(page, "/themes");
    await page.getByRole("tab", { name: "Site", exact: true }).click();
    await page
      .getByRole("button", { name: "Create theme", exact: true })
      .click();
    const mutation = (endpoint: keyof typeof THEME_ENDPOINTS) =>
      page.waitForResponse(
        (response) =>
          new URL(response.url()).pathname === THEME_ENDPOINTS[endpoint] &&
          response.request().method() === "POST",
      );
    await page
      .getByLabel("Name", { exact: true })
      .fill("Continuous shell proof");
    await page
      .getByLabel("Style (CSS)", { exact: true })
      .fill(":root { background: rgb(1, 2, 3); }");
    const [saved] = await Promise.all([
      mutation("save_editor"),
      page.getByRole("button", { name: "Save changes", exact: true }).click(),
    ]);
    expect(saved.status()).toBe(200);
    confirmedMutation(
      (await saved.json()) as MutationOutcome<unknown>,
      "themes::save_editor",
    );
    await page
      .getByRole("button", { name: "Edit Continuous shell proof", exact: true })
      .click();
    await openThemeAdvanced(page);
    await Promise.all([
      mutation("publish"),
      page.getByRole("button", { name: "Publish", exact: true }).click(),
    ]);
    const selection = page.getByLabel("Use published version", { exact: true });
    const id = await selection
      .getByRole("option", { name: "Continuous shell proof" })
      .getAttribute("value");
    expect(id).not.toBeNull();
    await mutate(async () => {
      const [response] = await Promise.all([
        mutation("select"),
        selection.selectOption(id!),
      ]);
      expect(response.status()).toBe(200);
      confirmedMutation(
        (await response.json()) as MutationOutcome<null>,
        "themes::select",
      );
    });

    const context = await tracedContext();
    try {
      const publicPage = await context.newPage();
      await goto(publicPage, "/");
      await expect(publicPage.locator(".j-root")).toHaveAttribute(
        "data-theme",
        "custom",
      );
      for (const selector of [".j-sidebar", ".j-scroll"]) {
        await expect(publicPage.locator(selector)).toHaveCSS(
          "background-color",
          "rgba(0, 0, 0, 0)",
        );
      }
    } finally {
      await context.close();
    }
  });
});
