import { test, expect } from "./fixtures";
import { goto, signInAs, signInAsNewUser } from "./helpers";
import { expectAccessible } from "./accessibility";

const ASSET_PATH = "assets/pixel.png";
const ASSET_BYTES = [
  137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0,
  0, 0, 1, 8, 2, 0, 0, 0, 144, 119, 83, 222, 0, 0, 0, 15, 73, 68, 65, 84, 120,
  1, 1, 4, 0, 251, 255, 0, 18, 52, 86, 0, 248, 0, 157, 248, 215, 100, 140, 0, 0,
  0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];
const THEME_ENDPOINTS = {
  import_css: "/api/themes/import_css",
  import_package: "/api/themes/import_package",
  import_zip: "/api/themes/import_zip",
  preview: "/api/themes/preview",
  publish: "/api/themes/publish",
  remove: "/api/themes/remove",
  rename: "/api/themes/rename",
  replace_binding: "/api/themes/replace_binding",
  replace_pool: "/api/themes/replace_pool",
  select: "/api/themes/select",
  shuffle: "/api/themes/shuffle",
} as const;
type ThemeEndpoint = keyof typeof THEME_ENDPOINTS;

// The Studio journey must remain a private surface: custom public presentation
// stylesheets may only appear in the isolated preview document, never in its parent.
test("theme management mounts as Studio without a public theme stylesheet", async ({
  page,
}) => {
  await signInAsNewUser(page);
  await goto(page, "/studio/themes");

  await expect(
    page.getByRole("heading", { name: "Catalog scope" }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Author catalog" }),
  ).toBeVisible();
  await expect(page.locator(".j-root")).toHaveAttribute("data-theme", "studio");
  await expect(page.locator("link[data-jaunder-theme-stylesheet]")).toHaveCount(
    0,
  );
  await expectAccessible(page);
});

test("author completes the custom theme lifecycle through Studio", async ({
  page,
  tracedContext,
}) => {
  const username = await signInAsNewUser(page);
  await goto(page, "/studio/themes");
  const mutation = (endpoint: ThemeEndpoint) =>
    page.waitForResponse(
      (response) =>
        new URL(response.url()).pathname === THEME_ENDPOINTS[endpoint] &&
        response.request().method() === "POST",
    );

  const cssImport = page.locator("section").filter({ hasText: "Import CSS" });
  await cssImport.getByLabel("Theme name").fill("Night author");
  await cssImport
    .getByLabel("Stylesheet")
    .fill("[data-jaunder-theme-surface] { outline: 2px solid rgb(1, 2, 3); }");
  await Promise.all([
    mutation("import_css"),
    cssImport.getByRole("button", { name: "Import CSS draft" }).click(),
  ]);

  await expect(
    page.getByRole("button", { name: /Night author/ }),
  ).toBeVisible();
  await page.getByRole("button", { name: /Night author/ }).click();
  await page.getByLabel("Rename selected theme").fill("Night round trip");
  await Promise.all([
    mutation("rename"),
    page.getByRole("button", { name: "Rename", exact: true }).click(),
  ]);
  await expect(
    page.getByRole("button", { name: /Night round trip/ }),
  ).toBeVisible();

  await page.getByLabel("Package assets JSON").fill("{");
  await page
    .getByRole("button", { name: "Save complete draft package" })
    .click();
  await expect(page.getByRole("status")).toContainText(
    "Package assets must be valid JSON",
  );

  await page.getByLabel("theme.json").fill(
    JSON.stringify({
      schema: 1,
      name: "Night round trip",
      style_contract: 1,
      assets: { [ASSET_PATH]: "image/png" },
      defaults: { logo: ASSET_PATH, header: [ASSET_PATH] },
    }),
  );
  await page
    .getByLabel("style.css")
    .fill("[data-jaunder-theme-surface] { color: rgb(1, 2, 3); }");
  await page
    .getByLabel("Package assets JSON")
    .fill(
      JSON.stringify([
        { path: ASSET_PATH, mime: "image/png", bytes: ASSET_BYTES },
      ]),
    );
  await Promise.all([
    mutation("import_package"),
    page.getByRole("button", { name: "Save complete draft package" }).click(),
  ]);

  await Promise.all([
    mutation("replace_binding"),
    page.getByRole("button", { name: "Use package logo default" }).click(),
  ]);
  await page.getByLabel("Header pool package asset paths").fill(ASSET_PATH);
  await Promise.all([
    mutation("replace_pool"),
    page.getByRole("button", { name: "Save header pool" }).click(),
  ]);
  await Promise.all([
    mutation("shuffle"),
    page.getByRole("button", { name: "Shuffle assignments" }).click(),
  ]);

  await Promise.all([
    mutation("preview"),
    page.getByRole("button", { name: "Preview draft" }).click(),
  ]);
  await expect(
    page.getByTitle("Isolated theme preview").contentFrame().locator("body"),
  ).toContainText("Jaunder");
  await page.setViewportSize({ width: 375, height: 800 });
  await expect(page.getByTitle("Isolated theme preview")).toBeVisible();
  await expect(page.locator("link[data-jaunder-theme-stylesheet]")).toHaveCount(
    0,
  );

  await Promise.all([
    mutation("publish"),
    page.getByRole("button", { name: "Publish" }).click(),
  ]);
  const publicSelection = page.getByLabel("Public selection");
  await expect(
    publicSelection.getByRole("option", { name: "Night round trip" }),
  ).toHaveCount(1);
  const themeId = await publicSelection
    .getByRole("option", { name: "Night round trip" })
    .getAttribute("value");
  expect(themeId).not.toBeNull();
  await Promise.all([
    mutation("select"),
    publicSelection.selectOption(themeId!),
  ]);

  const freshContext = await tracedContext();
  const freshPage = await freshContext.newPage();
  await signInAs(freshPage, username);
  await goto(freshPage, "/studio/themes");
  await expect(freshPage.getByLabel("Public selection")).toHaveValue(themeId!);
  await freshContext.close();

  const downloadPromise = page.waitForEvent("download");
  await page.getByRole("button", { name: "Export ZIP" }).click();
  const exported = await downloadPromise;
  const exportedPath = await exported.path();
  expect(exportedPath).not.toBeNull();

  await Promise.all([
    mutation("remove"),
    page.getByRole("button", { name: "Delete theme" }).click(),
  ]);
  await expect(
    page.getByRole("button", { name: /Night round trip/ }),
  ).toHaveCount(0);
  await expect(publicSelection).toHaveValue("inherit");

  const zipImport = page
    .locator("section")
    .filter({ hasText: "Import Theme Package" });
  await zipImport.getByLabel("Theme name").fill("Restored package");
  await zipImport.getByLabel("Theme Package ZIP").setInputFiles(exportedPath!);
  await Promise.all([
    mutation("import_zip"),
    zipImport.getByRole("button", { name: "Import ZIP draft" }).click(),
  ]);
  await expect(
    page.getByRole("button", { name: /Restored package/ }),
  ).toBeVisible();
});

test("operator can expose the distinct site catalog control", async ({
  page,
}) => {
  await signInAs(page, "testoperator");
  await goto(page, "/studio/themes");

  await expect(
    page.getByRole("button", { name: "Site catalog" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Site catalog" }).click();
  await expect(
    page.getByRole("button", { name: "Site catalog" }),
  ).toHaveAttribute("aria-pressed", "true");
});
