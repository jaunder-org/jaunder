import { test, expect } from "./fixtures";
import { goto } from "./helpers";

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
