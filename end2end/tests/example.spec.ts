import { test, expect } from "./fixtures";

test("homepage has title and links to intro page", async ({ page }) => {
  await page.goto("http://localhost:3000/");

  await expect(page).toHaveTitle("Jaunder");

  await expect(page.locator(".j-topbar h1")).toHaveText("jaunder.local");
  await expect(
    page.locator("main").getByRole("link", { name: "Sign in" }),
  ).toBeVisible();
  await expect(
    page.locator("main").getByRole("link", { name: "Register" }),
  ).toBeVisible();
});
