import { test, expect } from "./fixtures";
import { goto } from "./helpers";

test("homepage has title and links to intro page", async ({ page }) => {
  await goto(page, "/");

  await expect(page).toHaveTitle("Jaunder");

  await expect(page.locator(".j-topbar h1")).toHaveText("jaunder.local");
  await expect(
    page.locator("main").getByRole("link", { name: "Sign in" }),
  ).toBeVisible();
  await expect(
    page.locator("main").getByRole("link", { name: "Register" }),
  ).toBeVisible();
});
