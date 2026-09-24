import { test, expect } from "./fixtures";
import { expectAccessible } from "./accessibility";
import { BASE_URL, goto, signInAsNewUser } from "./helpers";
import { navigateInApp } from "./navigate";
import { createPostViaApi } from "./posts";
import { seedPostsViaTool } from "./seed";

const PAGE = '[data-test="manage-posts-page"]';
const ROW = '[data-test="managed-post"]';

async function expectNoHorizontalOverflow(
  page: import("@playwright/test").Page,
) {
  const sizes = await page.evaluate(() => ({
    clientWidth: document.documentElement.clientWidth,
    scrollWidth: document.documentElement.scrollWidth,
  }));
  expect(sizes.scrollWidth).toBeLessThanOrEqual(sizes.clientWidth);
}

test("Manage Posts selects across pages and applies atomic bulk operations", async ({
  page,
}) => {
  const username = await signInAsNewUser(page);
  await seedPostsViaTool(username, 52, "Managed Post", { published: false });

  await goto(page, "/posts/manage");
  await expect(page.locator(PAGE)).toBeVisible();
  await expect(page.locator(ROW)).toHaveCount(50);
  await expect(
    page.getByRole("link", { name: "Manage Posts" }),
  ).toHaveAttribute("href", "/posts/manage");
  const search = page.getByLabel("Search title or slug");
  await search.fill("  MANAGED  ");
  await page.getByRole("button", { name: "Apply filters" }).click();
  await expect(page.locator(ROW)).toHaveCount(50);
  await search.fill("does-not-exist");
  await page.getByRole("button", { name: "Apply filters" }).click();
  await expect(page.locator(ROW)).toHaveCount(0);
  await search.fill("");
  await page.getByRole("button", { name: "Apply filters" }).click();
  await expect(page.locator(ROW)).toHaveCount(50);

  await page.locator(ROW).first().getByRole("checkbox").check();
  await expect(page.getByText("1 selected", { exact: true })).toBeVisible();
  const firstPagePost = await page
    .locator(ROW)
    .first()
    .getAttribute("data-post-id");
  await page.getByRole("button", { name: "Next page" }).click();
  await expect
    .poll(() => page.locator(ROW).first().getAttribute("data-post-id"))
    .not.toBe(firstPagePost);
  await expect(page.locator(ROW)).toHaveCount(2);
  await page.locator(ROW).first().getByRole("checkbox").check();
  await expect(page.getByText("2 selected", { exact: true })).toBeVisible();

  await page.getByRole("button", { name: "Change audience" }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toContainText("exactly 2 Posts");
  await expect(dialog).toContainText("complete Audience Selection");
  await dialog.getByLabel("Audience").selectOption("subscribers");
  await dialog.getByRole("button", { name: "Confirm" }).click();
  await expect(page.getByRole("status")).toHaveText(
    "Selected 2 Posts; changed 2.",
  );
  const audienceFilter = page
    .locator(".j-manage-filters")
    .getByLabel("Audience");
  await audienceFilter.selectOption("subscribers");
  await expect(page.locator(ROW)).toHaveCount(2);
  await page.getByRole("button", { name: "Select all matching" }).click();
  await expect(page.getByText("2 selected", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Change audience" }).click();
  await dialog.getByLabel("Audience").selectOption("subscribers");
  await dialog.getByRole("button", { name: "Confirm" }).click();
  await expect(page.getByRole("status")).toHaveText(
    "Selected 2 Posts; changed 0.",
  );
  await audienceFilter.selectOption("all");
  await expect(page.locator(ROW)).toHaveCount(50);

  await page.getByRole("button", { name: "Select all matching" }).click();
  await expect(page.getByText("52 selected", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Delete" }).click();
  await expect(dialog).toContainText("exactly 52 Posts");
  const count = dialog.getByLabel("Enter 52 to confirm");
  await expect(dialog.getByRole("button", { name: "Confirm" })).toBeDisabled();
  await count.fill("52");
  await dialog.getByRole("button", { name: "Confirm" }).click();
  await expect(page.getByRole("status")).toHaveText(
    "Selected 52 Posts; changed 52.",
  );
  await expect(page.getByText("No Posts match these filters.")).toBeVisible();
  await expectNoHorizontalOverflow(page);
  await expectAccessible(page);
});

test("Manage Posts keeps a stale conflict actionable and reports no success", async ({
  registeredPage,
}) => {
  const page = await registeredPage("/app");
  const post = await createPostViaApi(page, {
    body: "# Conflict target\n\nBody",
    publish: false,
  });
  await navigateInApp(
    page,
    () => page.getByRole("link", { name: "Manage Posts" }).click(),
    { url: "/posts/manage", ready: PAGE },
  );
  const row = page.locator(`${ROW}[data-post-id="${post.post_id}"]`);
  await row.getByRole("checkbox").check();
  await page.getByRole("button", { name: "Delete" }).click();
  await expect(page.getByRole("dialog")).toContainText("exactly 1 Posts");

  const response = await page.request.post(`${BASE_URL}/api/posts/delete`, {
    form: { post_id: String(post.post_id) },
  });
  expect(response.ok(), await response.text()).toBeTruthy();
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Confirm" })
    .click();

  await expect(page.getByRole("alert")).toContainText("Selection changed");
  await expect(page.getByRole("status")).toHaveCount(0);
  await expect(page.getByRole("dialog")).toBeVisible();
});
