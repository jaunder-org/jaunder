import { test, expect } from "./fixtures";
import { goto, signInAs } from "./helpers";

const THEME_ENDPOINTS = {
  import_css: "/api/themes/import_css",
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
  await signInAs(page, "testoperator");
  await goto(page, "/themes");
  await page.getByRole("button", { name: "Site catalog" }).click();
  const mutation = (endpoint: keyof typeof THEME_ENDPOINTS) =>
    page.waitForResponse(
      (response) =>
        new URL(response.url()).pathname === THEME_ENDPOINTS[endpoint] &&
        response.request().method() === "POST",
    );
  const form = page.locator("section").filter({ hasText: "Import CSS" });
  await form.getByLabel("Theme name").fill("Continuous shell proof");
  await form
    .getByLabel("Stylesheet")
    .fill(":root { background: rgb(1, 2, 3); }");
  await Promise.all([
    mutation("import_css"),
    form.getByRole("button", { name: "Import CSS draft" }).click(),
  ]);
  await page.getByRole("button", { name: /Continuous shell proof/ }).click();
  await Promise.all([
    mutation("publish"),
    page.getByRole("button", { name: "Publish", exact: true }).click(),
  ]);
  const selection = page.getByLabel("Public selection");
  const id = await selection
    .getByRole("option", { name: "Continuous shell proof" })
    .getAttribute("value");
  expect(id).not.toBeNull();
  await Promise.all([mutation("select"), selection.selectOption(id!)]);

  const context = await tracedContext();
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
  await context.close();
});
