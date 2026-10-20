import type { Page } from "@playwright/test";
import { test, expect } from "./fixtures";
import { expectAccessible } from "./accessibility";
import {
  failServerFn,
  BASE_URL,
  confirmedMutation,
  type MutationOutcome,
  goto,
  signInAs,
  signInAsNewUser,
  stallServerFn,
} from "./helpers";
import { imageFixtureDirectory, uploadMedia } from "./media-helpers";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { createHash } from "node:crypto";
import {
  importMinimalTheme,
  minimalThemeZip,
  openThemeAdvanced,
  packageMember,
  packageMemberDigests,
  publishTheme,
  publishAndSelectTheme,
  withSiteThemeSelection,
} from "./theme-helpers";

const ASSET_PATH = "assets/pixel.png";
const ASSET_BYTES = [
  137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0,
  0, 0, 1, 8, 2, 0, 0, 0, 144, 119, 83, 222, 0, 0, 0, 15, 73, 68, 65, 84, 120,
  1, 1, 4, 0, 251, 255, 0, 18, 52, 86, 0, 248, 0, 157, 248, 215, 100, 140, 0, 0,
  0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];
const THEME_ENDPOINTS = {
  list: "/api/themes/list",
  get_selection: "/api/themes/get_selection",
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

// Compare the effective public revision, not an assumed Studio site default.
async function publicThemeIdentity(page: Page) {
  return {
    theme: await page.locator(".j-root").getAttribute("data-theme"),
    stylesheet: await page
      .locator("link[data-jaunder-theme-stylesheet]")
      .evaluateAll((links) => links.map((link) => link.getAttribute("href"))),
  };
}

async function wheelToEditorBottom(page: Page) {
  const scroll = page.locator(".j-scroll");
  const box = await scroll.boundingBox();
  expect(box).not.toBeNull();
  // Use the scroll canvas edge, outside nested source textareas. Firefox caps
  // each wheel event near one viewport, even when deltaY is much larger.
  await page.mouse.move(box!.x + box!.width - 4, box!.y + box!.height / 2);
  const pages = await scroll.evaluate((node) =>
    Math.ceil(node.scrollHeight / node.clientHeight),
  );
  for (let index = 0; index < pages; index += 1) {
    const before = await scroll.evaluate((node) => ({
      top: node.scrollTop,
      end: node.scrollTop + node.clientHeight >= node.scrollHeight - 1,
    }));
    if (before.end) break;
    await page.mouse.wheel(0, box!.height);
    await expect
      .poll(() => scroll.evaluate((node) => node.scrollTop))
      .toBeGreaterThan(before.top);
  }
}

test("publication rejection stops selection and offers draft repair without uncertain-success claims", async ({
  page,
}) => {
  await signInAsNewUser(page);
  await importMinimalTheme(page);
  let selections = 0;
  page.on("request", (request) => {
    if (new URL(request.url()).pathname === THEME_ENDPOINTS.select)
      selections += 1;
  });
  await page.route(`**${THEME_ENDPOINTS.publish}`, (route) =>
    route.fulfill({
      status: 500,
      contentType: "application/json",
      body: JSON.stringify({
        validation: { message: "Draft rejected for this test" },
      }),
    }),
  );
  await page
    .getByRole("button", { name: "Use this theme", exact: true })
    .click();
  await expect(page.getByRole("status")).toContainText(
    "Getting your theme ready was rejected",
  );
  await expect(page.getByRole("status")).toHaveClass(/error/);
  await expect(page.getByRole("status")).not.toHaveClass(/success/);
  await expect(page.getByRole("status")).toContainText("Fix the theme");
  await expect(page.getByRole("status")).not.toContainText("may have changed");
  await expect(
    page.getByRole("button", { name: "Use this theme", exact: true }),
  ).toBeEnabled();
  expect(selections).toBe(0);
  await expect(
    page.getByLabel("Current theme", { exact: true }),
  ).toHaveAttribute("data-theme-selection", "inherit");
});

for (const stage of ["publish", "select"] as const) {
  for (const fault of [
    "reject",
    "indeterminate",
    "lost",
    "undecodable",
  ] as const) {
    if (stage === "publish" && fault === "reject") continue; // Covered by the repair tracer above.
    test(`${stage} ${fault} stops or finishes chaining honestly and rereads without retry`, async ({
      page,
      tracedContext,
    }) => {
      const username = await signInAsNewUser(page);
      await importMinimalTheme(page);
      const baselineContext = await tracedContext();
      let baseline: Awaited<ReturnType<typeof publicThemeIdentity>>;
      try {
        const baselinePage = await baselineContext.newPage();
        await goto(baselinePage, `/~${username}`);
        baseline = await publicThemeIdentity(baselinePage);
      } finally {
        await baselineContext.close();
      }
      const writes: { path: string; body: string }[] = [];
      const reads: string[] = [];
      page.on("request", (request) => {
        const path = new URL(request.url()).pathname;
        if (path === THEME_ENDPOINTS.publish || path === THEME_ENDPOINTS.select)
          writes.push({ path, body: request.postData() ?? "" });
        if (path === "/api/themes/list" || path === "/api/themes/get_selection")
          reads.push(path);
      });
      await page.route(`**${THEME_ENDPOINTS[stage]}`, async (route) => {
        if (fault === "reject") {
          await route.fulfill({
            status: 500,
            contentType: "application/json",
            body: JSON.stringify({
              validation: { message: "Selection rejected for this test" },
            }),
          });
          return;
        }
        // Exercise the real mutation first; only its acknowledgement is replaced/lost.
        const response = await route.fetch();
        expect(response.status()).toBe(200);
        expect(await response.json()).toEqual({ Confirmed: null });
        if (fault === "lost") await route.abort("failed");
        else
          await route.fulfill({
            response,
            body:
              fault === "indeterminate"
                ? JSON.stringify({ CommitIndeterminate: null })
                : "undecodable acknowledgement",
          });
      });
      const use = page.getByRole("button", {
        name: "Use this theme",
        exact: true,
      });
      await use.click();
      const status = page.getByRole("status");
      await expect(status).toContainText(
        stage === "publish"
          ? "We could not confirm your theme is ready"
          : fault === "reject"
            ? "switching was rejected"
            : "switching outcome is uncertain",
      );
      await expect(status).not.toContainText("Your theme is now in use");
      await expect(status).toHaveClass(/error/);
      await expect(status).not.toHaveClass(/success/);
      await expect(status).toContainText("Available themes checked");
      await expect(status).toContainText("Current theme:");
      await expect(use).toBeEnabled();
      const expectedWrites =
        stage === "publish"
          ? [THEME_ENDPOINTS.publish]
          : [THEME_ENDPOINTS.publish, THEME_ENDPOINTS.select];
      expect(writes.map((write) => write.path)).toEqual(expectedWrites);
      expect(reads).toContain("/api/themes/list");
      expect(reads).toContain("/api/themes/get_selection");
      expect(new URLSearchParams(writes[0].body).get("scope")).toBe("author");
      if (stage === "select") {
        await expect(status).toContainText("Your updated theme is ready");
        const publish = new URLSearchParams(writes[0].body);
        const select = new URLSearchParams(writes[1].body);
        expect(select.get("scope")).toBe("author");
        expect(select.get("selection[value]")).toBe(publish.get("theme_id"));
      } else
        await expect(status).toContainText("We did not try to switch themes");
      const applied = stage === "select" && fault !== "reject";
      if (!applied)
        await expect(
          page.getByLabel("Current theme", { exact: true }),
        ).toHaveAttribute("data-theme-selection", "inherit");
      else
        await expect(
          page.getByLabel("Current theme", { exact: true }),
        ).not.toHaveAttribute("data-theme-selection", "inherit");
      const publicContext = await tracedContext();
      try {
        const publicPage = await publicContext.newPage();
        await goto(publicPage, `/~${username}`);
        if (!applied)
          expect(await publicThemeIdentity(publicPage)).toEqual(baseline!);
        else
          await expect(publicPage.locator(".j-root")).toHaveAttribute(
            "data-theme",
            "custom",
          );
        if (applied)
          await expect(
            publicPage.locator('[data-jaunder-part="main"]'),
          ).toHaveCSS("outline-color", "rgb(1, 2, 3)");
      } finally {
        await publicContext.close();
      }
      await expect(page.locator(".j-root")).toHaveAttribute(
        "data-theme",
        "studio",
      );
      await expect(
        page.locator("link[data-jaunder-theme-stylesheet]"),
      ).toHaveCount(0);
    });
  }

  test(`${stage} uncertainty reports failed catalog and selection rereads as unknown`, async ({
    page,
  }) => {
    await signInAsNewUser(page);
    await importMinimalTheme(page);
    let dispatched = false;
    const writes: string[] = [];
    page.on("request", (request) => {
      const path = new URL(request.url()).pathname;
      if (path === THEME_ENDPOINTS.publish || path === THEME_ENDPOINTS.select)
        writes.push(path);
    });
    for (const endpoint of [
      THEME_ENDPOINTS.list,
      THEME_ENDPOINTS.get_selection,
    ]) {
      await page.route(`**${endpoint}`, async (route) => {
        if (dispatched)
          await route.fulfill({ status: 500, body: "reread unavailable" });
        else await route.continue();
      });
    }
    await page.route(`**${THEME_ENDPOINTS[stage]}`, async (route) => {
      const response = await route.fetch();
      expect(response.status()).toBe(200);
      expect(await response.json()).toEqual({ Confirmed: null });
      dispatched = true;
      await route.fulfill({
        response,
        body: JSON.stringify({ CommitIndeterminate: null }),
      });
    });
    const use = page.getByRole("button", {
      name: "Use this theme",
      exact: true,
    });
    await use.click();
    await expect(page.getByRole("status")).toContainText(
      "Theme readiness is unknown",
    );
    await expect(page.getByRole("status")).toContainText(
      "current setting is unknown",
    );
    await expect(page.getByRole("status")).toContainText("reopen Themes");
    await expect(page.getByRole("status")).not.toContainText(
      "Current theme: Using the site theme",
    );
    await expect(use).toBeEnabled();
    await expect(page.getByLabel("Use a saved version")).toHaveCount(0);
    await expect(
      page.getByRole("button", { name: "Use site theme", exact: true }),
    ).toBeDisabled();
    await expect(
      page.getByLabel("Current theme", { exact: true }),
    ).toHaveAttribute("data-theme-selection", "");
    await expect(page.getByLabel("Current theme", { exact: true })).toHaveText(
      "Current theme could not be checked",
    );
    await expect(
      page.getByText(
        "Current theme could not be checked. This choice may already be in use.",
      ),
    ).toBeVisible();
    await expect(
      page.getByText("Saved, not live yet", { exact: true }),
    ).toHaveCount(0);
    expect(writes).toEqual(
      stage === "publish"
        ? [THEME_ENDPOINTS.publish]
        : [THEME_ENDPOINTS.publish, THEME_ENDPOINTS.select],
    );
  });
}

test("invalid package assets after successful import gives error feedback outside Advanced in the catalog", async ({
  page,
}) => {
  await signInAsNewUser(page);
  await importMinimalTheme(page);
  const zip = page.locator(
    'section[aria-labelledby="theme-package-import-heading"]',
  );
  await expect(zip.getByRole("status")).toContainText(
    "Uploaded Intake fixture",
  );
  await expect(zip.getByRole("status")).toHaveClass(/success/);
  await expect(zip.getByRole("status")).not.toHaveClass(/error/);
  await openThemeAdvanced(page);
  await page.getByLabel("Package assets JSON").fill("not JSON");
  await page
    .getByRole("button", { name: "Save complete draft package", exact: true })
    .click();
  const status = page.getByRole("status");
  await expect(status).toContainText("Package assets must be valid JSON");
  await expect(status).toHaveClass(/error/);
  await expect(status).not.toHaveClass(/success/);
  const catalog = page.locator(
    'section[aria-labelledby="theme-catalog-heading"]',
  );
  await expect(catalog.getByRole("status")).toContainText(
    "Package assets must be valid JSON",
  );
  await expect(zip.getByRole("status")).toHaveCount(0);
  expect(
    await status.evaluate((node) =>
      Boolean(node.closest("#theme-advanced-editor")),
    ),
  ).toBe(false);
});

test("pending Advanced draft write blocks use, duplicate write and scope changes", async ({
  page,
}) => {
  await signInAsNewUser(page);
  await importMinimalTheme(page);
  await openThemeAdvanced(page);
  await page
    .getByLabel("Stylesheet source")
    .fill('[data-jaunder-part="main"] { outline: 2px solid rgb(4, 5, 6); }');
  const release = await stallServerFn(page, "themes/replace_css");
  const writes: string[] = [];
  page.on("request", (request) => {
    const path = new URL(request.url()).pathname;
    if (
      [
        THEME_ENDPOINTS.publish,
        THEME_ENDPOINTS.select,
        "/api/themes/replace_css",
      ].includes(path)
    )
      writes.push(path);
  });
  const save = page.getByRole("button", { name: "Save CSS", exact: true });
  await save.click();
  const use = page.getByRole("button", { name: "Use this theme", exact: true });
  const scope = page.getByRole("button", {
    name: "Use site theme",
    exact: true,
  });
  try {
    await expect(use).toBeDisabled();
    await expect(save).toBeDisabled();
    await expect(page.getByRole("status")).toContainText("Working");
    await expect(page.getByRole("status")).not.toHaveClass(/success|error/);
    await expect(scope).toBeDisabled();
    await save.evaluate((node) =>
      node.dispatchEvent(new MouseEvent("click", { bubbles: true })),
    );
    await use.evaluate((node) =>
      node.dispatchEvent(new MouseEvent("click", { bubbles: true })),
    );
    expect(writes).toEqual(["/api/themes/replace_css"]);
  } finally {
    release();
  }
  await expect(page.getByRole("status")).toContainText("Saved");
  await expect(page.getByRole("status")).toHaveClass(/success/);
  await expect(page.getByRole("status")).not.toHaveClass(/error/);
  await expect(use).toBeEnabled();
  await expect(save).toBeEnabled();
  expect(writes).toEqual(["/api/themes/replace_css"]);
});

for (const operation of ["preview", "export"] as const) {
  test(`Advanced ${operation} failure is live, actionable, and does not publish`, async ({
    page,
  }) => {
    await signInAsNewUser(page);
    await importMinimalTheme(page);
    await openThemeAdvanced(page);
    await failServerFn(page, `themes/${operation}`);
    const writes: string[] = [];
    page.on("request", (request) => {
      const path = new URL(request.url()).pathname;
      if (path === THEME_ENDPOINTS.publish || path === THEME_ENDPOINTS.select)
        writes.push(path);
    });
    await page
      .getByRole("button", {
        name: operation === "preview" ? "Preview draft" : "Export ZIP",
        exact: true,
      })
      .click();
    const status = page.getByRole("status");
    await expect(status).toContainText(
      operation === "preview" ? "Preview failed" : "Export failed",
    );
    await expect(status).toContainText("reopen Themes", { ignoreCase: true });
    await expect(status).toHaveClass(/error/);
    await expect(
      page.getByRole("button", { name: "Use this theme", exact: true }),
    ).toBeEnabled();
    expect(writes).toEqual([]);
  });
}

test("already-selected publication advances the public revision even when applying is rejected", async ({
  page,
  tracedContext,
}) => {
  const username = await signInAsNewUser(page);
  // Establish the selected revision directly, without spending this owner's
  // operation burst on exporter/importer setup unrelated to republication.
  const selected = String(await publishAndSelectTheme(page));
  await goto(page, "/themes");
  await expect(
    page.getByLabel("Current theme", { exact: true }),
  ).toHaveAttribute("data-theme-selection", selected);
  await page.getByRole("button", { name: "Conformance", exact: true }).click();
  const use = page.getByRole("button", { name: "Use this theme", exact: true });
  await openThemeAdvanced(page);
  await page
    .getByLabel("style.css", { exact: true })
    .fill('[data-jaunder-part="main"] { outline: 2px solid rgb(4, 5, 6); }');
  await Promise.all([
    page.waitForResponse(
      (response) =>
        new URL(response.url()).pathname === THEME_ENDPOINTS.import_package,
    ),
    page
      .getByRole("button", { name: "Save complete draft package", exact: true })
      .click(),
  ]);
  await expect(page.getByRole("status")).toContainText("Saved");
  await page.route(`**${THEME_ENDPOINTS.select}`, (route) =>
    route.fulfill({
      status: 500,
      contentType: "application/json",
      body: JSON.stringify({
        validation: { message: "Applying rejected for this test" },
      }),
    }),
  );
  await use.click();
  await expect(page.getByRole("status")).toContainText(
    "Your updated theme is ready, but switching was rejected",
  );
  await expect(page.getByRole("status")).toContainText(
    "already in use, the new version is already live",
  );
  await expect(use).toBeEnabled();
  await expect(
    page.getByLabel("Current theme", { exact: true }),
  ).toHaveAttribute("data-theme-selection", selected);
  const publicContext = await tracedContext();
  try {
    const publicPage = await publicContext.newPage();
    await goto(publicPage, `/~${username}`);
    await expect(publicPage.locator('[data-jaunder-part="main"]')).toHaveCSS(
      "outline-color",
      "rgb(4, 5, 6)",
    );
  } finally {
    await publicContext.close();
  }
});

test("ZIP-only import opens an unselected draft and explicit use publishes then applies it", async ({
  page,
  tracedContext,
}) => {
  const username = await signInAsNewUser(page);
  const archive = await minimalThemeZip(page);
  await goto(page, "/themes");
  await expect(
    page.getByRole("heading", { level: 1, name: "User theme", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText(
      "Choose how visitors see your public pages. This does not change your private workspace.",
    ),
  ).toBeVisible();
  await expect(page.getByLabel("Current theme", { exact: true })).toHaveText(
    "Using the site theme",
  );
  await expect(page.getByRole("group", { name: "Theme views" })).toHaveCount(0);
  await expect(
    page.getByRole("heading", { name: "Add or choose a theme", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Review your choice", exact: true }),
  ).toBeVisible();
  await expect(page.getByLabel("Stylesheet", { exact: true })).toBeHidden();
  await expect(page.getByLabel("Theme name (optional)")).toBeHidden();
  await expect(
    page.getByRole("button", { name: "Advanced", exact: true }),
  ).toHaveAttribute("aria-expanded", "false");
  await expect(
    page.getByRole("button", { name: "Site theme", exact: true }),
  ).toHaveCount(0);
  const zipImport = page.locator(
    "section[aria-labelledby='theme-package-import-heading']",
  );
  await zipImport.getByLabel("Theme ZIP file").setInputFiles({
    name: "filename-disagrees.zip",
    mimeType: "application/zip",
    buffer: archive,
  });
  const releaseImport = await stallServerFn(page, "themes/import_zip");
  await zipImport.getByRole("button", { name: "Upload theme" }).click();
  try {
    await expect(page.getByRole("status")).toContainText("Uploading");
    await expect(page.getByRole("status")).not.toHaveClass(/success|error/);
  } finally {
    releaseImport();
  }
  const entry = page.getByRole("button", {
    name: "Intake fixture",
    exact: true,
  });
  await expect(entry).toBeVisible();
  await expect(page.getByRole("status")).toHaveClass(/success/);
  await expect(page.getByRole("status")).not.toHaveClass(/error/);
  const selection = page.getByLabel("Use a saved version");
  await expect(
    page.getByLabel("Current theme", { exact: true }),
  ).toHaveAttribute("data-theme-selection", "inherit");
  await entry.click();
  await expect(entry).toHaveAttribute("aria-pressed", "true");
  await expect(
    page.getByRole("heading", { name: "Intake fixture", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText("Saved, not live yet", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByLabel("Current theme", { exact: true }),
  ).toHaveAttribute("data-theme-selection", "inherit");
  await expect(page.getByLabel("theme.json")).toBeHidden();
  const release = await stallServerFn(page, "themes/publish");
  const releaseApply = await stallServerFn(page, "themes/select");
  const requests: string[] = [];
  page.on("request", (request) => {
    const path = new URL(request.url()).pathname;
    if (path === THEME_ENDPOINTS.publish || path === THEME_ENDPOINTS.select)
      requests.push(path);
  });
  const use = page.getByRole("button", { name: "Use this theme", exact: true });
  await use.click();
  await expect(page.getByRole("status")).toContainText(
    "Getting your theme ready",
  );
  await expect(page.getByRole("status")).not.toHaveClass(/success|error/);
  await expect(use).toBeDisabled();
  await expect(selection).toHaveCount(0);
  await expect(entry).toBeDisabled();
  await expect(
    page.getByRole("button", { name: "Use site theme", exact: true }),
  ).toBeDisabled();
  expect(requests).toEqual([THEME_ENDPOINTS.publish]);
  release();
  try {
    await expect(page.getByRole("status")).toContainText("Applying your theme");
    await expect(page.getByRole("status")).not.toHaveClass(/success|error/);
  } finally {
    releaseApply();
  }
  await expect(page.getByRole("status")).toContainText(
    "Your theme is now in use",
  );
  await expect(page.getByRole("status")).toHaveClass(/success/);
  await expect(page.getByRole("status")).not.toHaveClass(/error/);
  expect(requests).toEqual([THEME_ENDPOINTS.publish, THEME_ENDPOINTS.select]);
  await expect(
    page.getByLabel("Current theme", { exact: true }),
  ).not.toHaveAttribute("data-theme-selection", "inherit");
  await expect(page.getByLabel("Current theme", { exact: true })).toHaveText(
    "Intake fixture",
  );
  await expect(page.getByText(/^Currently in use\./)).toBeVisible();
  await expect(
    page.getByText("Saved, not live yet", { exact: true }),
  ).toHaveCount(0);
  expect(
    await page
      .getByRole("heading", { name: "Review your choice", exact: true })
      .evaluate((node) => {
        const add = document.querySelector("#theme-add-heading");
        const use = Array.from(document.querySelectorAll("button")).find(
          (button) => button.textContent === "Use this theme",
        );
        return Boolean(
          add &&
          use &&
          add.compareDocumentPosition(node) &
            Node.DOCUMENT_POSITION_FOLLOWING &&
          node.compareDocumentPosition(use) & Node.DOCUMENT_POSITION_FOLLOWING,
        );
      }),
  ).toBe(true);
  await expect(page.locator(".j-root")).toHaveAttribute("data-theme", "studio");
  await expect(page.locator("link[data-jaunder-theme-stylesheet]")).toHaveCount(
    0,
  );
  const publicContext = await tracedContext();
  try {
    const publicPage = await publicContext.newPage();
    await goto(publicPage, `/~${username}`);
    await expect(publicPage.locator(".j-root")).toHaveAttribute(
      "data-theme",
      "custom",
    );
    await expect(publicPage.locator('[data-jaunder-part="main"]')).toHaveCSS(
      "outline-color",
      "rgb(1, 2, 3)",
    );
  } finally {
    await publicContext.close();
  }
});

test("Advanced preserves unsaved input across toggles and hides collapsed controls", async ({
  page,
}) => {
  await signInAs(page, "testoperator");
  const archive = await minimalThemeZip(page);
  await goto(page, "/themes");
  await page.getByLabel("Theme ZIP file").setInputFiles({
    name: "fixture.zip",
    mimeType: "application/zip",
    buffer: archive,
  });
  await page.getByRole("button", { name: "Upload theme" }).click();
  await expect(
    page.getByRole("button", { name: "Intake fixture", exact: true }),
  ).toBeVisible();
  await openThemeAdvanced(page);
  const create = page.locator(
    'section[aria-labelledby="theme-create-heading"]',
  );
  const cssImport = page.locator(
    'section[aria-labelledby="theme-import-heading"]',
  );
  const fields = [
    [page.getByLabel("theme.json", { exact: true }), '{"unsaved":"manifest"}'],
    [
      page.getByLabel("style.css", { exact: true }),
      "/* unsaved package CSS */",
    ],
    [page.getByLabel("Stylesheet source"), "/* unsaved replacement CSS */"],
    [
      page.getByLabel("Rename selected theme", { exact: true }),
      "Unsaved rename",
    ],
    [create.getByLabel("Theme name"), "Unsaved empty draft"],
    [cssImport.getByLabel("Theme name"), "Unsaved CSS import"],
    [
      cssImport.getByLabel("Stylesheet", { exact: true }),
      "/* unsaved import CSS */",
    ],
    [page.getByLabel("Theme name (optional)"), "Second draft"],
  ] as const;
  for (const [field, value] of fields) await field.fill(value);
  const advanced = page.getByRole("button", { name: "Advanced", exact: true });
  await advanced.click();
  await expect(advanced).toHaveAttribute("aria-expanded", "false");
  await advanced.click();
  for (const [field, value] of fields) await expect(field).toHaveValue(value);
  await advanced.click();
  await expect(advanced).toHaveAttribute("aria-expanded", "false");
  for (const [field] of fields) {
    await expect(field).toBeHidden();
    expect(
      await field.evaluate((node) => {
        (node as HTMLElement).focus();
        return document.activeElement === node;
      }),
    ).toBe(false);
  }
  await advanced.focus();
  await page.keyboard.press("Tab");
  expect(
    await page.evaluate(() =>
      Boolean(document.activeElement?.closest("[hidden]")),
    ),
  ).toBe(false);
  await expect(
    page.getByRole("button", { name: "Use this theme", exact: true }),
  ).toBeVisible();
  await openThemeAdvanced(page);
  // Imported draft identity, unlike disclosure visibility, is a reset boundary.
  await page.getByRole("button", { name: "Upload theme" }).click();
  await expect(
    page.getByRole("button", { name: "Second draft", exact: true }),
  ).toBeVisible();
  await expect(page.getByLabel("style.css", { exact: true })).toHaveValue(
    '[data-jaunder-part="main"] { outline: 2px solid rgb(1, 2, 3); }',
  );
  await expect(
    page.getByLabel("Rename selected theme", { exact: true }),
  ).toHaveValue("");
  await page.getByRole("button", { name: "Site theme", exact: true }).click();
  await expect(page.getByLabel("Theme name (optional)")).toHaveValue("");
  await expect(cssImport.getByLabel("Theme name")).toHaveValue("");
});

test("blank Advanced catalog override uses the package name", async ({
  page,
}) => {
  await signInAsNewUser(page);
  const archive = await minimalThemeZip(page);
  await goto(page, "/themes");
  await openThemeAdvanced(page);
  await page.getByLabel("Theme name (optional)").fill("   ");
  await page.getByLabel("Theme ZIP file").setInputFiles({
    name: "different-filename.zip",
    mimeType: "application/zip",
    buffer: archive,
  });
  await page.getByRole("button", { name: "Upload theme" }).click();
  await expect(
    page.getByRole("button", { name: "Intake fixture", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByLabel("Current theme", { exact: true }),
  ).toHaveAttribute("data-theme-selection", "inherit");
});

test("missing or malformed ZIP gives nearby actionable feedback without import or selection", async ({
  page,
}) => {
  await signInAsNewUser(page);
  await goto(page, "/themes");
  const zip = page.locator(
    'section[aria-labelledby="theme-package-import-heading"]',
  );
  let imports = 0;
  page.on("request", (request) => {
    if (new URL(request.url()).pathname === THEME_ENDPOINTS.import_zip)
      imports += 1;
  });
  await zip.getByRole("button", { name: "Upload theme", exact: true }).click();
  await expect(zip.getByRole("status")).toContainText(
    "Choose a theme ZIP file first",
  );
  expect(imports).toBe(0);
  await zip.getByLabel("Theme ZIP file").setInputFiles({
    name: "not-a-package.zip",
    mimeType: "application/zip",
    buffer: Buffer.from("not a ZIP package"),
  });
  await zip.getByRole("button", { name: "Upload theme", exact: true }).click();
  await expect(zip.getByRole("status")).toContainText("Upload rejected");
  await expect(zip.getByRole("status")).toContainText(
    "Choose a valid theme ZIP file",
  );
  await expect(zip.getByRole("status")).toHaveAttribute("aria-live", "polite");
  await expect(zip.getByRole("status")).not.toContainText("Uploaded");
  await expect(
    zip.getByRole("button", { name: "Upload theme", exact: true }),
  ).toBeEnabled();
  expect(imports).toBe(1);
  await expect(page.locator(".j-theme-catalog-item")).toHaveCount(0);
  await expect(
    page.getByLabel("Current theme", { exact: true }),
  ).toHaveAttribute("data-theme-selection", "inherit");
});

for (const manifestName of ["Studio", "x".repeat(101)]) {
  test(`catalog-invalid package name ${manifestName.length === 101 ? "too long" : "reserved"} offers an explicit override`, async ({
    page,
  }) => {
    await signInAsNewUser(page);
    const archive = await minimalThemeZip(page, manifestName);
    await goto(page, "/themes");
    const zip = page.locator(
      'section[aria-labelledby="theme-package-import-heading"]',
    );
    await zip.getByLabel("Theme ZIP file").setInputFiles({
      name: "filename-is-not-a-fallback.zip",
      mimeType: "application/zip",
      buffer: archive,
    });
    await zip
      .getByRole("button", { name: "Upload theme", exact: true })
      .click();
    await expect(zip.getByRole("status")).toContainText("Upload rejected");
    await expect(zip.getByRole("status")).toContainText("Advanced");
    await expect(page.locator(".j-theme-catalog-item")).toHaveCount(0);
    await openThemeAdvanced(page);
    await expect(zip.getByRole("status")).toContainText("Upload rejected");
    await expect(zip.getByRole("status")).toHaveClass(/error/);
    await page.getByLabel("Theme name (optional)").fill("Studio");
    await zip
      .getByRole("button", { name: "Upload theme", exact: true })
      .click();
    await expect(zip.getByRole("status")).toContainText("Upload rejected");
    await expect(page.locator(".j-theme-catalog-item")).toHaveCount(0);
    await page.getByLabel("Theme name (optional)").fill("Recovered label");
    await zip
      .getByRole("button", { name: "Upload theme", exact: true })
      .click();
    await expect(
      page.getByRole("button", {
        name: "Recovered label",
        exact: true,
      }),
    ).toBeVisible();
    await expect(zip.getByRole("status")).toContainText(
      "Uploaded Recovered label",
    );
    await expect(
      page.getByLabel("Current theme", { exact: true }),
    ).toHaveAttribute("data-theme-selection", "inherit");
  });
}

test("duplicate theme name rejects without suffix and explicit override recovers", async ({
  page,
}) => {
  await signInAsNewUser(page);
  await importMinimalTheme(page);
  const zip = page.locator(
    'section[aria-labelledby="theme-package-import-heading"]',
  );
  await zip.getByRole("button", { name: "Upload theme", exact: true }).click();
  await expect(zip.getByRole("status")).toContainText("Upload rejected");
  await expect(zip.getByRole("status")).toContainText("duplicate theme name");
  await expect(page.locator(".j-theme-catalog-item")).toHaveCount(1);
  await openThemeAdvanced(page);
  await expect(zip.getByRole("status")).toContainText("Upload rejected");
  await expect(zip.getByRole("status")).toHaveClass(/error/);
  await page.getByLabel("Theme name (optional)").fill("Distinct label");
  await zip.getByRole("button", { name: "Upload theme", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Distinct label", exact: true }),
  ).toBeVisible();
  await expect(page.locator(".j-theme-catalog-item")).toHaveCount(2);
  await expect(
    page.getByLabel("Current theme", { exact: true }),
  ).toHaveAttribute("data-theme-selection", "inherit");
});

for (const viewport of [
  { width: 390, height: 844 },
  { width: 1440, height: 900 },
]) {
  test(`Advanced keyboard disclosure and ordinary-wheel bottom reachability at ${viewport.width}x${viewport.height}`, async ({
    page,
  }) => {
    await page.setViewportSize(viewport);
    await page.emulateMedia({ colorScheme: "light", reducedMotion: "reduce" });
    await signInAsNewUser(page);
    await importMinimalTheme(page);
    expect(await page.evaluate(() => window.devicePixelRatio)).toBe(1);
    const advanced = page.getByRole("button", {
      name: "Advanced",
      exact: true,
    });
    const manifest = page.getByLabel("theme.json", { exact: true });
    await expect(advanced).toHaveAttribute("aria-expanded", "false");
    await expect(manifest).toBeHidden();
    expect(
      await manifest.evaluate((node) => {
        (node as HTMLElement).focus();
        return document.activeElement === node;
      }),
    ).toBe(false);
    await advanced.focus();
    await page.keyboard.press("Enter");
    await expect(advanced).toHaveAttribute("aria-expanded", "true");
    await expect(advanced).toBeFocused();
    await manifest.fill('{"unsaved":"reachable"}');
    await advanced.focus();
    await page.keyboard.press("Space");
    await expect(manifest).toBeHidden();
    await page.keyboard.press("Enter");
    await expect(manifest).toHaveValue('{"unsaved":"reachable"}');
    await expectAccessible(page);
    // Actual wheel input, not locator clicks/scrollIntoView, must expose the editor's last control.
    await wheelToEditorBottom(page);
    const bottom = page.getByRole("button", {
      name: "Delete theme",
      exact: true,
    });
    await expect(bottom).toBeInViewport({ ratio: 1 });
    expect(
      await page.locator(".j-scroll").evaluate((node) => node.scrollTop),
    ).toBeGreaterThan(0);
    // Starting at the disclosure, native Tab walks to the bottom and scrolls focus into view.
    await advanced.focus();
    let reached = false;
    for (let index = 0; index < 60; index += 1) {
      await page.keyboard.press("Tab");
      if (await bottom.evaluate((node) => document.activeElement === node)) {
        reached = true;
        break;
      }
      expect(
        await page.evaluate(() =>
          Boolean(document.activeElement?.closest("[hidden]")),
        ),
      ).toBe(false);
    }
    expect(reached).toBe(true);
    await expect(bottom).toBeFocused();
    await expect(bottom).toBeInViewport({ ratio: 1 });
    await advanced.focus();
    await page.keyboard.press("Enter");
    const use = page.getByRole("button", {
      name: "Use this theme",
      exact: true,
    });
    const release = await stallServerFn(page, "themes/publish");
    await use.focus();
    await page.keyboard.press("Enter");
    const status = page.getByRole("status");
    await expect(status).toContainText("Getting your theme ready");
    await expect(status).toHaveAttribute("aria-live", "polite");
    expect(
      await status.evaluate((node) =>
        Boolean(
          node.closest(
            '[id="theme-advanced-editor"], [id="theme-advanced-import"]',
          ),
        ),
      ),
    ).toBe(false);
    release();
    await expect(status).toContainText("Your theme is now in use");
    await expect(use).toBeEnabled();
    await expect(status).toBeInViewport();
    await page.route(`**${THEME_ENDPOINTS.publish}`, (route) =>
      route.fulfill({
        status: 500,
        contentType: "application/json",
        body: JSON.stringify({ validation: { message: "Draft needs repair" } }),
      }),
    );
    await use.click();
    await expect(status).toContainText("Getting your theme ready was rejected");
    await expect(status).toContainText("Fix the theme");
    await expect(status).toHaveClass(/error/);
    await expect(use).toBeEnabled();
    await page.mouse.move(viewport.width - 40, 350);
    await page.mouse.wheel(0, 20000);
    await expect(status).toBeInViewport({ ratio: 1 });
    expect(
      await page.evaluate(() =>
        Boolean(document.activeElement?.closest("[hidden]")),
      ),
    ).toBe(false);
  });
}

// The Studio journey must remain a private surface: custom public presentation
// stylesheets may only appear in the isolated preview document, never in its parent.
test("theme management mounts as Studio without a public theme stylesheet", async ({
  page,
}) => {
  await signInAsNewUser(page);
  await goto(page, "/themes");

  await openThemeAdvanced(page);
  const guide = page.getByRole("link", {
    name: "Read the theme repository guide",
  });
  await expect(guide).toHaveAttribute(
    "href",
    "https://github.com/jaunder-org/jaunder/blob/main/docs/themes.md",
  );
  await expect(guide).toHaveAttribute("target", "_blank");
  await expect(
    page.getByText(
      "No saved themes yet. Upload a theme ZIP file to get started.",
    ),
  ).toBeVisible();
  await expect(page.locator(".j-loading")).toHaveCount(0);
  await expect(page.locator(".j-theme-catalog-item")).toHaveCount(0);
  await expect(
    page.getByRole("heading", { level: 1, name: "User theme", exact: true }),
  ).toBeVisible();
  await expect(page.getByRole("group", { name: "Theme views" })).toHaveCount(0);
  await expect(page.locator(".j-root")).toHaveAttribute("data-theme", "studio");
  await expect(page.locator("link[data-jaunder-theme-stylesheet]")).toHaveCount(
    0,
  );
  for (const viewport of [
    { width: 1280, height: 800 },
    { width: 390, height: 844 },
  ]) {
    await page.setViewportSize(viewport);
    const guidance = page.getByRole("region", {
      name: "Author themes from a repository",
    });
    const scope = page.getByRole("region", {
      name: "Current theme setting",
      exact: true,
    });
    const guidanceBox = await guidance.boundingBox();
    const scopeBox = await scope.boundingBox();
    const scopeHeadingBox = await scope
      .getByRole("heading", { name: "Current theme", exact: true })
      .boundingBox();
    const scopeButtonBox = await scope
      .getByLabel("Current theme", { exact: true })
      .boundingBox();
    expect(guidanceBox).not.toBeNull();
    expect(scopeBox).not.toBeNull();
    expect(scopeHeadingBox).not.toBeNull();
    expect(scopeButtonBox).not.toBeNull();
    const headingInset = scopeHeadingBox!.x - scopeBox!.x;
    const bodyInset = scopeButtonBox!.x - scopeBox!.x;
    expect(headingInset).toBeGreaterThan(0);
    expect(bodyInset).toBeGreaterThan(0);

    for (const [content, inset] of [
      [guidance.getByRole("heading"), headingInset],
      [guidance.locator("p"), headingInset],
      [guide, bodyInset],
    ] as const) {
      await expect(content).toBeVisible();
      const contentBox = await content.boundingBox();
      expect(contentBox).not.toBeNull();
      expect(contentBox!.x - guidanceBox!.x).toBeCloseTo(inset, 0);
      expect(contentBox!.y).toBeGreaterThan(guidanceBox!.y);
      expect(contentBox!.x + contentBox!.width).toBeLessThanOrEqual(
        guidanceBox!.x + guidanceBox!.width - inset,
      );
      expect(contentBox!.y + contentBox!.height).toBeLessThan(
        guidanceBox!.y + guidanceBox!.height,
      );
    }
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= window.innerWidth,
      ),
    ).toBe(true);
    await expectAccessible(page);
  }
});

test("theme catalog loading does not offer the empty-catalog next action", async ({
  page,
}) => {
  await signInAsNewUser(page);
  const release = await stallServerFn(page, "themes/list");
  await goto(page, "/themes");

  await expect(page.getByText("Loading available themes…")).toBeVisible();
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
  await openThemeAdvanced(page);
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
  const uploadedLogo = await uploadMedia(
    page,
    "blog-logo.png",
    Buffer.from(ASSET_BYTES),
    "image/png",
  );
  // Owned BEFORE inputs differ in retained color controls, not private metadata.
  const headerOneInput = readFileSync(
    resolve(imageFixtureDirectory, "png-original.png"),
  );
  const headerTwoInput = readFileSync(
    resolve(imageFixtureDirectory, "png-color-original.png"),
  );
  const headerOne = await uploadMedia(
    page,
    "header-one.png",
    headerOneInput,
    "image/png",
  );
  const headerTwo = await uploadMedia(
    page,
    "header-two.png",
    headerTwoInput,
    "image/png",
  );
  // Theme bindings must retain these exact prepared identities and public bytes.
  const headerOneResponse = await page.request.get(BASE_URL + headerOne.url);
  const headerTwoResponse = await page.request.get(BASE_URL + headerTwo.url);
  expect(headerOneResponse.status()).toBe(200);
  expect(headerTwoResponse.status()).toBe(200);
  const headerOneBytes = await headerOneResponse.body();
  const headerTwoBytes = await headerTwoResponse.body();
  const headerOneHash = createHash("sha256")
    .update(headerOneBytes)
    .digest("hex");
  const headerTwoHash = createHash("sha256")
    .update(headerTwoBytes)
    .digest("hex");
  expect(headerOneHash).not.toBe(headerTwoHash);
  expect(headerOne.url).toContain(headerOneHash);
  expect(headerTwo.url).toContain(headerTwoHash);
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

  await openThemeAdvanced(page);
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

  const packageManifest = {
    assets: { [ASSET_PATH]: "image/png" },
    defaults: { header: [ASSET_PATH], logo: ASSET_PATH },
    name: "Night round trip",
    schema: 1,
    style_contract: 1,
  };
  const packageStylesheet =
    "[data-jaunder-theme-surface] { color: rgb(1, 2, 3); }";
  await page.getByLabel("theme.json").fill(JSON.stringify(packageManifest));
  await page.getByLabel("style.css").fill(packageStylesheet);
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
  await page.getByLabel("Header pool package asset paths").fill(ASSET_PATH);
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
    ASSET_PATH,
  );
  await Promise.all([
    mutation("replace_pool"),
    page.getByRole("button", { name: "Save header pool" }).click(),
  ]);
  await expect(page.getByLabel("Header pool package asset paths")).toHaveValue(
    ASSET_PATH,
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
  await expect(page.getByRole("status")).toHaveClass(/success/);
  await expect(page.getByRole("status")).not.toHaveClass(/error/);
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
  await expect(page.getByRole("status")).toContainText("Publication confirmed");
  await expect(page.getByRole("status")).toHaveClass(/success/);
  await expect(page.getByRole("status")).not.toHaveClass(/error/);
  const publicSelection = page.getByLabel("Use a saved version");
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
  await expect(
    page.getByLabel("Current theme", { exact: true }),
  ).toHaveAttribute("data-theme-selection", themeId!);

  const publicContext = await tracedContext();
  try {
    const publicPage = await publicContext.newPage();
    await goto(publicPage, `/~${username}`);
    const logo = publicPage.locator('[data-jaunder-part="logo"]');
    await expect(logo).toBeVisible();
    const logoUrl = await logo.getAttribute("src");
    expect(logoUrl).toBe(uploadedLogo.url);
    const publicLogo = await publicPage.request.get(
      new URL(logoUrl!, publicPage.url()).toString(),
    );
    expect(publicLogo.status()).toBe(200);
    expect(await publicLogo.body()).toEqual(Buffer.from(ASSET_BYTES));
    const header = publicPage.locator('[data-jaunder-part="header-image"]');
    await expect(header).toBeVisible();
    const headerUrl = await header.getAttribute("src");
    expect(headerUrl).not.toBeNull();
    const headerPath = new URL(headerUrl!, publicPage.url()).pathname;
    const ownedHeaderBytes = new Map([
      [new URL(headerOne.url, publicPage.url()).pathname, headerOneBytes],
      [new URL(headerTwo.url, publicPage.url()).pathname, headerTwoBytes],
    ]);
    const expectedHeaderBytes = headerPath.startsWith("/theme/")
      ? Buffer.from(ASSET_BYTES)
      : ownedHeaderBytes.get(headerPath);
    expect(expectedHeaderBytes).toBeDefined();
    const publicHeader = await publicPage.request.get(
      new URL(headerUrl!, publicPage.url()).toString(),
    );
    expect(publicHeader.status()).toBe(200);
    expect(await publicHeader.body()).toEqual(expectedHeaderBytes);
  } finally {
    await publicContext.close();
  }

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
  await openThemeAdvanced(freshPage);
  await expect(freshPage.getByLabel("Use a saved version")).toHaveValue(
    themeId!,
  );
  await expect(
    freshPage.getByLabel("Current theme", { exact: true }),
  ).toHaveAttribute("data-theme-selection", themeId!);
  await freshPage.getByRole("button", { name: /Night round trip/ }).click();
  await openThemeAdvanced(freshPage);
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
  await expect(page.getByRole("status")).toContainText("download started");
  await expect(page.getByRole("status")).toHaveClass(/success/);
  await expect(page.getByRole("status")).not.toHaveClass(/error/);
  const exportedPath = await exported.path();
  expect(exportedPath).not.toBeNull();
  const exportedMembers = packageMemberDigests(exportedPath!);
  expect([...exportedMembers.keys()]).toEqual([
    ASSET_PATH,
    "style.css",
    "theme.json",
  ]);
  expect(packageMember(exportedPath!, "theme.json")).toEqual(
    Buffer.from(JSON.stringify(packageManifest)),
  );
  expect(packageMember(exportedPath!, "style.css")).toEqual(
    Buffer.from(packageStylesheet),
  );
  expect(packageMember(exportedPath!, ASSET_PATH)).toEqual(
    Buffer.from(ASSET_BYTES),
  );
  expect(exportedMembers.has("blog-logo.png")).toBe(false);
  expect(exportedMembers.has("header-one.png")).toBe(false);
  expect(exportedMembers.has("header-two.png")).toBe(false);

  await Promise.all([
    mutation("remove"),
    page.getByRole("button", { name: "Delete theme" }).click(),
  ]);
  await expect(
    page.getByRole("button", { name: /Night round trip/ }),
  ).toHaveCount(0);
  await expect(
    page.getByLabel("Current theme", { exact: true }),
  ).toHaveAttribute("data-theme-selection", "inherit");

  const zipImport = page.locator(
    'section[aria-labelledby="theme-package-import-heading"]',
  );
  await zipImport.getByLabel("Theme name (optional)").fill("Restored package");
  await zipImport.getByLabel("Theme ZIP file").setInputFiles(exportedPath!);
  await Promise.all([
    mutation("import_zip"),
    zipImport.getByRole("button", { name: "Upload theme" }).click(),
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

test("current theme and Advanced saved choices stay truthful across inheritance and bundled recovery", async ({
  page,
  tracedContext,
}) => {
  const username = await signInAsNewUser(page);
  const themeId = String(await publishTheme(page));
  await goto(page, "/themes");
  const saved = page.getByLabel("Use a saved version");
  await expect(
    saved.getByRole("option", { name: "Conformance", includeHidden: true }),
  ).toHaveAttribute("value", themeId);
  // A real placeholder must be selected, not selectedIndex=-1 from an unsupported token.
  await expect
    .poll(() =>
      saved.evaluate((node: HTMLSelectElement) =>
        Array.from(node.selectedOptions, (option) => option.value),
      ),
    )
    .toEqual([""]);
  await expect(saved).toBeHidden();
  await expect(page.getByLabel("Current theme", { exact: true })).toHaveText(
    "Using the site theme",
  );
  expect(
    await page.getByLabel("Current theme", { exact: true }).evaluate((node) => {
      const add = document.querySelector("#theme-add-heading");
      const review = document.querySelector("#theme-catalog-heading");
      const other = document.querySelector("#theme-ready-heading");
      return Boolean(
        add &&
        review &&
        other &&
        node.compareDocumentPosition(add) & Node.DOCUMENT_POSITION_FOLLOWING &&
        add.compareDocumentPosition(review) &
          Node.DOCUMENT_POSITION_FOLLOWING &&
        review.compareDocumentPosition(other) &
          Node.DOCUMENT_POSITION_FOLLOWING,
      );
    }),
  ).toBe(true);
  await expect(
    page.getByRole("heading", { name: "Other theme choices", exact: true }),
  ).toBeVisible();
  await openThemeAdvanced(page);
  await expect(saved).toBeVisible();
  await expect(saved.locator("option:checked")).toHaveText(
    "Choose a ready saved theme",
  );
  const writes: { path: string; body: string }[] = [];
  page.on("request", (request) => {
    const path = new URL(request.url()).pathname;
    if (path === THEME_ENDPOINTS.publish || path === THEME_ENDPOINTS.select)
      writes.push({ path, body: request.postData() ?? "" });
  });
  await saved.selectOption(themeId);
  await expect(
    page.getByLabel("Current theme", { exact: true }),
  ).toHaveAttribute("data-theme-selection", themeId);
  expect(writes.map((write) => write.path)).toEqual([THEME_ENDPOINTS.select]);
  const selected = new URLSearchParams(writes[0].body);
  expect(selected.get("scope")).toBe("author");
  expect(selected.get("selection[value]")).toBe(themeId);
  await page.getByRole("button", { name: "Use reader", exact: true }).click();
  await expect(page.getByLabel("Current theme", { exact: true })).toHaveText(
    "reader",
  );
  await expect
    .poll(() =>
      saved.evaluate((node: HTMLSelectElement) =>
        Array.from(node.selectedOptions, (option) => option.value),
      ),
    )
    .toEqual([""]);
  await page
    .getByRole("button", { name: "Use site theme", exact: true })
    .click();
  await expect(page.getByLabel("Current theme", { exact: true })).toHaveText(
    "Using the site theme",
  );
  await expect(saved.locator("option:checked")).toHaveText(
    "Choose a ready saved theme",
  );
  await saved.selectOption(themeId);
  await expect(saved).toHaveValue(themeId);
  await expect(
    page.getByLabel("Current theme", { exact: true }),
  ).toHaveAttribute("data-theme-selection", themeId);
  expect(writes.map((write) => write.path)).toEqual(
    Array(4).fill(THEME_ENDPOINTS.select),
  );
  const context = await tracedContext();
  try {
    const cold = await context.newPage();
    await signInAs(cold, username);
    const release = await stallServerFn(cold, "themes/list");
    await goto(cold, "/themes");
    await expect(cold.getByLabel("Use a saved version")).toHaveCount(0);
    release();
    await expect(cold.getByLabel("Use a saved version")).toBeHidden();
    await openThemeAdvanced(cold);
    await expect(cold.getByLabel("Use a saved version")).toHaveValue(themeId);
  } finally {
    await context.close();
  }
});

test("cold Studio return retains a custom author selection before built-in recovery", async ({
  page,
  tracedContext,
}) => {
  // A fresh owner avoids retained catalog labels across retries/other specs.
  const username = await signInAsNewUser(page);
  const baselineContext = await tracedContext();
  let baseline: Awaited<ReturnType<typeof publicThemeIdentity>>;
  try {
    const baselinePage = await baselineContext.newPage();
    await goto(baselinePage, `/~${username}`);
    baseline = await publicThemeIdentity(baselinePage);
  } finally {
    await baselineContext.close();
  }
  const themeId = await publishTheme(page);
  await goto(page, "/themes");
  await expect(page.getByLabel("Use a saved version")).toBeHidden();
  await openThemeAdvanced(page);

  const selection = page.getByLabel("Use a saved version");
  await expect(
    selection.getByRole("option", { name: "Conformance" }),
  ).toHaveAttribute("value", String(themeId));
  await Promise.all([
    page.waitForResponse(
      (response) =>
        new URL(response.url()).pathname === THEME_ENDPOINTS.select &&
        response.request().method() === "POST",
    ),
    selection.selectOption(String(themeId)),
  ]);
  await expect(
    page.getByLabel("Current theme", { exact: true }),
  ).toHaveAttribute("data-theme-selection", String(themeId));

  const publicContext = await tracedContext();
  try {
    const publicPage = await publicContext.newPage();
    await goto(publicPage, `/~${username}`);
    await expect(publicPage.locator(".j-root")).toHaveAttribute(
      "data-theme",
      "custom",
    );
  } finally {
    await publicContext.close();
  }

  const studioContext = await tracedContext();
  try {
    const studioPage = await studioContext.newPage();
    await signInAs(studioPage, username);
    await goto(studioPage, "/themes");
    await expect(studioPage.getByLabel("Use a saved version")).toBeHidden();
    await openThemeAdvanced(studioPage);
    const coldSelection = studioPage.getByLabel("Use a saved version");
    await expect(
      coldSelection.getByRole("option", { name: "Conformance" }),
    ).toHaveAttribute("value", String(themeId));
    await expect(coldSelection).toHaveValue(String(themeId));
    await expect(
      studioPage.getByLabel("Current theme", { exact: true }),
    ).toHaveAttribute("data-theme-selection", String(themeId));

    const recoveryWrites: string[] = [];
    studioPage.on("request", (request) => {
      const path = new URL(request.url()).pathname;
      if (path === THEME_ENDPOINTS.publish || path === THEME_ENDPOINTS.select)
        recoveryWrites.push(path);
    });
    await Promise.all([
      studioPage.waitForResponse(
        (response) =>
          new URL(response.url()).pathname === THEME_ENDPOINTS.select &&
          response.request().method() === "POST",
      ),
      studioPage
        .getByRole("button", { name: "Use studio", exact: true })
        .click(),
    ]);
    await expect(
      studioPage.getByLabel("Current theme", { exact: true }),
    ).toHaveAttribute("data-theme-selection", "studio");
    expect(recoveryWrites).toEqual([THEME_ENDPOINTS.select]);
    await studioPage
      .getByRole("button", { name: "Use site theme", exact: true })
      .click();
    await expect(
      studioPage.getByLabel("Current theme", { exact: true }),
    ).toHaveAttribute("data-theme-selection", "inherit");
    expect(recoveryWrites).toEqual([
      THEME_ENDPOINTS.select,
      THEME_ENDPOINTS.select,
    ]);
  } finally {
    await studioContext.close();
  }

  const recoveryContext = await tracedContext();
  try {
    const recoveryPage = await recoveryContext.newPage();
    await goto(recoveryPage, `/~${username}`);
    expect(await publicThemeIdentity(recoveryPage)).toEqual(baseline!);
  } finally {
    await recoveryContext.close();
  }
});

test("operator manages the site catalog through public selection and fallback", async ({
  page,
  tracedContext,
}) => {
  await withSiteThemeSelection(tracedContext, async (mutate) => {
    await signInAs(page, "testoperator");
    await goto(page, "/themes");
    const mutation = (endpoint: ThemeEndpoint) =>
      page.waitForResponse(
        (response) =>
          new URL(response.url()).pathname === THEME_ENDPOINTS[endpoint] &&
          response.request().method() === "POST",
      );

    const siteCatalog = page.getByRole("button", {
      name: "Site theme",
      exact: true,
    });
    await expect(siteCatalog).toBeVisible();
    await siteCatalog.click();
    await expect(siteCatalog).toHaveAttribute("aria-pressed", "true");
    await expect(
      page.getByRole("heading", { level: 1, name: "Site theme", exact: true }),
    ).toBeVisible();
    await expect(
      page.getByText(
        "Choose the default look for public site pages and user pages using the site theme.",
      ),
    ).toBeVisible();
    await expect(
      page.getByRole("button", { name: "Use site theme", exact: true }),
    ).toHaveCount(0);

    await openThemeAdvanced(page);
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

    const writes: { path: string; body: string }[] = [];
    page.on("request", (request) => {
      const path = new URL(request.url()).pathname;
      if (path === THEME_ENDPOINTS.publish || path === THEME_ENDPOINTS.select)
        writes.push({ path, body: request.postData() ?? "" });
    });
    const releasePublication = await stallServerFn(page, "themes/publish");
    const releaseSelection = await stallServerFn(page, "themes/select");
    const use = page.getByRole("button", {
      name: "Use this theme",
      exact: true,
    });
    const authorScope = page.getByRole("button", {
      name: "User theme",
      exact: true,
    });
    const publicSelection = page.getByLabel("Use a saved version");
    await mutate(async () => {
      const publicationResponse = page.waitForResponse(
        (response) =>
          new URL(response.url()).pathname === THEME_ENDPOINTS.publish &&
          response.request().method() === "POST",
      );
      const selectionResponse = page.waitForResponse(
        (response) =>
          new URL(response.url()).pathname === THEME_ENDPOINTS.select &&
          response.request().method() === "POST",
      );
      await use.click();
      await expect(page.getByRole("status")).toContainText(
        "Getting your theme ready",
      );
      await expect(authorScope).toBeDisabled();
      await expect(siteCatalog).toBeDisabled();
      await expect(publicSelection).toBeDisabled();
      // Even duplicate DOM delivery cannot dispatch twice or change the captured owner boundary.
      await use.evaluate((node) =>
        node.dispatchEvent(new MouseEvent("click", { bubbles: true })),
      );
      await authorScope.evaluate((node) =>
        node.dispatchEvent(new MouseEvent("click", { bubbles: true })),
      );
      expect(writes.map((write) => write.path)).toEqual([
        THEME_ENDPOINTS.publish,
      ]);
      await expect(siteCatalog).toHaveAttribute("aria-pressed", "true");
      releasePublication();
      await expect(page.getByRole("status")).toContainText(
        "Applying your theme",
      );
      await expect(use).toBeDisabled();
      await expect(authorScope).toBeDisabled();
      await use.evaluate((node) =>
        node.dispatchEvent(new MouseEvent("click", { bubbles: true })),
      );
      await authorScope.evaluate((node) =>
        node.dispatchEvent(new MouseEvent("click", { bubbles: true })),
      );
      releaseSelection();
      await expect(page.getByRole("status")).toContainText(
        "Your theme is now in use",
      );
      const publishedResponse = await publicationResponse;
      const selectedResponse = await selectionResponse;
      expect(publishedResponse.status()).toBe(200);
      expect(selectedResponse.status()).toBe(200);
      confirmedMutation(
        (await publishedResponse.json()) as MutationOutcome<null>,
        "themes::publish",
      );
      confirmedMutation(
        (await selectedResponse.json()) as MutationOutcome<null>,
        "themes::select",
      );
    });
    await expect(use).toBeEnabled();
    expect(writes.map((write) => write.path)).toEqual([
      THEME_ENDPOINTS.publish,
      THEME_ENDPOINTS.select,
    ]);
    const published = new URLSearchParams(writes[0].body);
    const applied = new URLSearchParams(writes[1].body);
    expect(published.get("scope")).toBe("site");
    expect(applied.get("scope")).toBe("site");
    expect(applied.get("selection[value]")).toBe(published.get("theme_id"));
    const themeId = await publicSelection
      .getByRole("option", { name: "Operator site published" })
      .getAttribute("value");
    expect(themeId).not.toBeNull();
    await expect(
      page.getByLabel("Current theme", { exact: true }),
    ).toHaveAttribute("data-theme-selection", themeId!);

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
      await expect(
        publicPage.locator("[data-jaunder-theme-surface]"),
      ).toHaveCSS("outline-color", "rgb(4, 5, 6)");
      const inheritingAuthor = await publicContext.newPage();
      await goto(inheritingAuthor, "/~testlogin");
      await expect(inheritingAuthor.locator(".j-root")).toHaveAttribute(
        "data-theme",
        "custom",
      );
      await expect(
        inheritingAuthor.locator("[data-jaunder-theme-surface]"),
      ).toHaveCSS("outline-color", "rgb(4, 5, 6)");
      await expect(
        publicPage.getByRole("group", { name: "Theme views" }),
      ).toHaveCount(0);
      await expect(
        publicPage.getByRole("button", { name: "User theme" }),
      ).toHaveCount(0);
    } finally {
      await publicContext.close();
    }

    await mutate(async () => {
      const [response] = await Promise.all([
        mutation("remove"),
        page.getByRole("button", { name: "Delete theme" }).click(),
      ]);
      expect(response.status()).toBe(200);
      confirmedMutation(
        (await response.json()) as MutationOutcome<null>,
        "themes::remove",
      );
    });
    await expect(
      page.getByRole("button", { name: /Operator site published/ }),
    ).toHaveCount(0);
    await expect(
      page.getByLabel("Current theme", { exact: true }),
    ).toHaveAttribute("data-theme-selection", "studio");

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
      ).toHaveCount(1);
      await expect(
        fallbackPage.locator("link[data-jaunder-theme-stylesheet]"),
      ).toHaveAttribute("href", /^\/theme\/[0-9a-f]{64}$/);
    } finally {
      await fallbackContext.close();
    }
  });
});
