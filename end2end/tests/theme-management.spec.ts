import { test, expect } from "./fixtures";
import { expectAccessible } from "./accessibility";
import {
  failServerFn,
  goto,
  signInAs,
  signInAsNewUser,
  stallServerFn,
} from "./helpers";
import { uploadMedia } from "./media-helpers";

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
  await goto(page, "/themes");

  const guide = page.getByRole("link", {
    name: "Read the theme repository guide",
  });
  await expect(guide).toHaveAttribute(
    "href",
    "https://github.com/jaunder-org/jaunder/blob/main/docs/themes.md",
  );
  await expect(
    page.getByText(
      "No themes in this catalog yet. Create a draft or import a package to start authoring.",
    ),
  ).toBeVisible();
  await expect(page.locator(".j-loading")).toHaveCount(0);
  await expect(page.locator(".j-theme-catalog-item")).toHaveCount(0);
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

test("theme catalog loading does not offer the empty-catalog next action", async ({
  page,
}) => {
  await signInAsNewUser(page);
  const release = await stallServerFn(page, "themes/list");
  await goto(page, "/themes");

  await expect(page.getByText("Loading catalog…")).toBeVisible();
  await expect(page.locator(".j-theme-catalog-empty")).toHaveCount(0);

  release();
  await expect(page.locator(".j-theme-catalog-empty")).toBeVisible();
});

test("theme catalog server failure does not offer the empty-catalog next action", async ({
  page,
}) => {
  await signInAsNewUser(page);
  await failServerFn(page, "themes/list");
  await goto(page, "/themes");

  await expect(page.locator("p.error")).toBeVisible();
  await expect(page.locator(".j-theme-catalog-empty")).toHaveCount(0);
});

test("invalid authored CSS is rejected before draft persistence", async ({
  page,
}) => {
  await signInAsNewUser(page);
  await goto(page, "/themes");
  const cssImport = page.locator("section").filter({ hasText: "Import CSS" });
  await cssImport.getByLabel("Theme name").fill("Rejected external");
  await cssImport
    .getByLabel("Stylesheet")
    .fill(
      ".j-post { background-image: url(https://example.invalid/image.png); }",
    );

  await Promise.all([
    page.waitForResponse(
      (response) =>
        new URL(response.url()).pathname === THEME_ENDPOINTS.import_css &&
        response.request().method() === "POST",
    ),
    cssImport.getByRole("button", { name: "Import CSS draft" }).click(),
  ]);

  await expect(page.getByRole("status")).toContainText("invalid theme package");
  await expect(
    page.getByRole("button", { name: /Rejected external/ }),
  ).toHaveCount(0);
});

test("author completes the custom theme lifecycle through Studio", async ({
  page,
  tracedContext,
}) => {
  const username = await signInAsNewUser(page);
  await uploadMedia(
    page,
    "blog-logo.png",
    Buffer.from(ASSET_BYTES),
    "image/png",
  );
  await uploadMedia(
    page,
    "header-one.png",
    Buffer.from([...ASSET_BYTES.slice(0, -12), 1, ...ASSET_BYTES.slice(-11)]),
    "image/png",
  );
  await uploadMedia(
    page,
    "header-two.png",
    Buffer.from([...ASSET_BYTES.slice(0, -12), 2, ...ASSET_BYTES.slice(-11)]),
    "image/png",
  );
  for (let index = 0; index < 50; index += 1) {
    await uploadMedia(
      page,
      `newer-${index.toString().padStart(2, "0")}.png`,
      Buffer.from(ASSET_BYTES),
      "image/png",
    );
  }
  await goto(page, "/themes");
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

  await expect(page.getByRole("button", { name: "Next images" })).toBeEnabled();
  await page.getByRole("button", { name: "Next images" }).click();
  await expect(page.getByText("Image page 2")).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Next images" }),
  ).toBeDisabled();
  await Promise.all([
    mutation("replace_binding"),
    page
      .getByLabel("Logo image")
      .selectOption({ label: "Media: blog-logo.png" }),
  ]);
  await page.getByLabel("Header pool package asset paths").fill("");
  const mediaToAdd = page.getByLabel("Media to add");
  for (const filename of ["header-one.png", "header-two.png"]) {
    await mediaToAdd.selectOption({ label: filename });
    await page.getByRole("button", { name: "Add Media" }).click();
  }
  await expect(
    page.getByRole("list", { name: "Selected header Media" }),
  ).toContainText("header-one.png");
  await expect(
    page.getByRole("list", { name: "Selected header Media" }),
  ).toContainText("header-two.png");
  await Promise.all([
    mutation("replace_binding"),
    page.getByLabel("Logo image").selectOption("none"),
  ]);
  await expect(
    page.getByRole("list", { name: "Selected header Media" }),
  ).toContainText("header-one.png");
  await expect(
    page.getByRole("list", { name: "Selected header Media" }),
  ).toContainText("header-two.png");
  await Promise.all([
    mutation("replace_binding"),
    page
      .getByLabel("Logo image")
      .selectOption({ label: "Media: blog-logo.png" }),
  ]);
  await expect(page.getByLabel("Header pool package asset paths")).toHaveValue(
    "",
  );
  await Promise.all([
    mutation("replace_pool"),
    page.getByRole("button", { name: "Save header pool" }).click(),
  ]);
  await expect(page.getByLabel("Header pool package asset paths")).toHaveValue(
    "",
  );
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
  const desktopViewport = page.viewportSize();
  if (desktopViewport === null) {
    throw new Error("Studio lifecycle requires an explicit desktop viewport");
  }
  await page.setViewportSize({ width: 375, height: 800 });
  await expect(page.getByTitle("Isolated theme preview")).toBeVisible();
  await page.setViewportSize(desktopViewport);
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
  await expect(publicSelection).toHaveValue(themeId!);

  const freshContext = await tracedContext();
  const freshPage = await freshContext.newPage();
  await signInAs(freshPage, username);
  const releaseCatalog = await stallServerFn(freshPage, "themes/list");
  const persistedSelection = freshPage.waitForResponse(
    (response) =>
      new URL(response.url()).pathname === "/api/themes/get_selection",
  );
  await goto(freshPage, "/themes");
  await persistedSelection;
  releaseCatalog();
  await expect(freshPage.getByLabel("Public selection")).toHaveValue(themeId!);
  await freshPage.getByRole("button", { name: /Night round trip/ }).click();
  await expect(freshPage.getByLabel("Logo image")).toHaveValue(
    "unavailable-media",
  );
  await freshPage.getByRole("button", { name: "Next images" }).click();
  await expect(freshPage.getByLabel("Logo image")).toHaveValue(
    /media\/upload\//,
  );
  await expect(
    freshPage.getByRole("list", { name: "Selected header Media" }),
  ).toContainText("header-one.png");
  await expect(
    freshPage.getByRole("list", { name: "Selected header Media" }),
  ).toContainText("header-two.png");
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
  await page.getByRole("button", { name: /Restored package/ }).click();
  await expect(page.getByLabel("Logo image")).toHaveValue("package-default");
  await expect(
    page
      .getByRole("list", { name: "Selected header Media" })
      .getByRole("listitem"),
  ).toHaveCount(0);
});

test("operator manages the site catalog through public selection and fallback", async ({
  page,
  tracedContext,
}) => {
  await signInAs(page, "testoperator");
  await goto(page, "/themes");
  const mutation = (endpoint: ThemeEndpoint) =>
    page.waitForResponse(
      (response) =>
        new URL(response.url()).pathname === THEME_ENDPOINTS[endpoint] &&
        response.request().method() === "POST",
    );

  const siteCatalog = page.getByRole("button", { name: "Site catalog" });
  await expect(siteCatalog).toBeVisible();
  await siteCatalog.click();
  await expect(siteCatalog).toHaveAttribute("aria-pressed", "true");

  const cssImport = page.locator("section").filter({ hasText: "Import CSS" });
  await cssImport.getByLabel("Theme name").fill("Operator site draft");
  await cssImport
    .getByLabel("Stylesheet")
    .fill(":root { outline: 2px solid rgb(4, 5, 6); }");
  await Promise.all([
    mutation("import_css"),
    cssImport.getByRole("button", { name: "Import CSS draft" }).click(),
  ]);

  await page.getByRole("button", { name: /Operator site draft/ }).click();
  await page
    .getByLabel("Rename selected theme")
    .fill("Operator site published");
  await Promise.all([
    mutation("rename"),
    page.getByRole("button", { name: "Rename", exact: true }).click(),
  ]);
  await expect(
    page.getByRole("button", { name: /Operator site published/ }),
  ).toBeVisible();

  await Promise.all([
    mutation("preview"),
    page.getByRole("button", { name: "Preview draft" }).click(),
  ]);
  await expect(
    page.getByTitle("Isolated theme preview").contentFrame().locator("body"),
  ).toContainText("Jaunder");

  await Promise.all([
    mutation("publish"),
    page.getByRole("button", { name: "Publish", exact: true }).click(),
  ]);
  const publicSelection = page.getByLabel("Public selection");
  const selectedOption = publicSelection.getByRole("option", {
    name: "Operator site published",
  });
  await expect(selectedOption).toHaveCount(1);
  const themeId = await selectedOption.getAttribute("value");
  expect(themeId).not.toBeNull();
  await Promise.all([
    mutation("select"),
    publicSelection.selectOption(themeId!),
  ]);
  await expect(publicSelection).toHaveValue(themeId!);

  const publicContext = await tracedContext();
  try {
    const publicPage = await publicContext.newPage();
    await goto(publicPage, "/");
    await expect(publicPage.locator(".j-root")).toHaveAttribute(
      "data-theme",
      "custom",
    );
    await expect(
      publicPage.locator("link[data-jaunder-theme-stylesheet]"),
    ).toHaveCount(1);
    await expect(publicPage.locator("[data-jaunder-theme-surface]")).toHaveCSS(
      "outline-color",
      "rgb(4, 5, 6)",
    );
    await expect(
      publicPage.getByRole("radiogroup", { name: "Theme catalog scope" }),
    ).toHaveCount(0);
    await expect(
      publicPage.getByRole("button", { name: "Author catalog" }),
    ).toHaveCount(0);
  } finally {
    await publicContext.close();
  }

  await Promise.all([
    mutation("remove"),
    page.getByRole("button", { name: "Delete theme" }).click(),
  ]);
  await expect(
    page.getByRole("button", { name: /Operator site published/ }),
  ).toHaveCount(0);
  await expect(publicSelection).toHaveValue("studio");

  const fallbackContext = await tracedContext();
  try {
    const fallbackPage = await fallbackContext.newPage();
    await goto(fallbackPage, "/");
    await expect(fallbackPage.locator(".j-root")).toHaveAttribute(
      "data-theme",
      "studio",
    );
    await expect(
      fallbackPage.locator("link[data-jaunder-theme-stylesheet]"),
    ).toHaveCount(0);
  } finally {
    await fallbackContext.close();
  }
});
