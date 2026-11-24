import { seedConfigViaTool } from "./seed";
import { test, expect } from "./fixtures";
import { goto } from "./helpers";
import { SEL } from "./selectors";

test("Local has its masthead actions without a promotional hero", async ({
  page,
}) => {
  await seedConfigViaTool("site.title", "Jaunder");
  await seedConfigViaTool("site.registration_policy", "open");
  await goto(page, "/");

  await expect(page).toHaveTitle("Jaunder");

  await expect(page.locator(SEL.topbarHeading)).toHaveText("Jaunder");
  await expect(page.locator(".j-hero")).toHaveCount(0);
  await expect(page.getByText("One timeline. Every protocol.")).toHaveCount(0);
  await expect(
    page.locator("main").getByRole("link", { name: "Sign in" }),
  ).toBeVisible();
  await expect(
    page.locator("main").getByRole("link", { name: "Register" }),
  ).toBeVisible();
});
