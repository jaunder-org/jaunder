import { test, expect } from "./fixtures";
import { goto } from "./helpers";
import { SEL } from "./selectors";

test("homepage has title and links to intro page", async ({ page }) => {
  await goto(page, "/");

  await expect(page).toHaveTitle("Jaunder");

  await expect(page.locator(SEL.topbarHeading)).toHaveText("jaunder.local");
  await expect(
    page.locator("main").getByRole("link", { name: "Sign in" }),
  ).toBeVisible();
  await expect(
    page.locator("main").getByRole("link", { name: "Register" }),
  ).toBeVisible();
});
