import { test, expect } from "@playwright/test";

test("homepage has title and links to intro page", async ({ page }) => {
  await page.goto("http://localhost:3000/");

  await expect(page).toHaveTitle("Jaunder");

  await expect(page.locator("h1")).toHaveText("Jaunder");
  await expect(
    page.locator("main").getByRole("link", { name: "Login" }),
  ).toBeVisible();
  await expect(
    page.locator("main").getByRole("link", { name: "Register" }),
  ).toBeVisible();
});
