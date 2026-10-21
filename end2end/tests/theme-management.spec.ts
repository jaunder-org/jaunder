import type { Locator, Page } from "@playwright/test";
import { test, expect } from "./fixtures";
import { expectAccessible } from "./accessibility";
import { navigateInApp } from "./navigate";
import {
  BASE_URL,
  confirmedMutation,
  failServerFn,
  goto,
  signInAs,
  signInAsNewUser,
  stallServerFn,
  type MutationOutcome,
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

const THEME_ENDPOINTS = {
  list: "/api/themes/list",
  get_selection: "/api/themes/get_selection",
  import_package: "/api/themes/import_package",
  import_zip: "/api/themes/import_zip",
  copy_theme: "/api/themes/copy_theme",
  preview: "/api/themes/preview_editor",
  publish: "/api/themes/publish",
  remove: "/api/themes/remove",
  select: "/api/themes/select",
  save: "/api/themes/save_editor",
  pool: "/api/themes/replace_pool",
  shuffle: "/api/themes/shuffle",
};
type ThemeEndpoint = keyof typeof THEME_ENDPOINTS;
const css = (r: number, g: number, b: number) =>
  `[data-jaunder-part="main"] { outline: 2px solid rgb(${r}, ${g}, ${b}); }`;
const normal = (page: Page) =>
  page.getByRole("form", { name: "Theme editor", exact: true });
const status = (page: Page) =>
  page.locator(".j-theme-feedback").getByRole("status");
const current = (page: Page) =>
  page.getByLabel("Current theme", { exact: true });
const response = (page: Page, endpoint: ThemeEndpoint) =>
  page.waitForResponse(
    (r) =>
      new URL(r.url()).pathname === THEME_ENDPOINTS[endpoint] &&
      r.request().method() === "POST",
  );
async function save(page: Page) {
  const pending = response(page, "save");
  await page.getByRole("button", { name: "Save changes", exact: true }).click();
  const result = await pending;
  expect(result.status()).toBe(200);
  const saved = confirmedMutation(
    (await result.json()) as MutationOutcome<{
      id: number;
      name: string;
      applied: boolean;
    }>,
    "themes::save_editor",
  );
  await expect(status(page)).toHaveText(
    saved.applied
      ? "Changes saved. Public appearance updated."
      : "Changes saved.",
  );
  return saved;
}
async function createLocal(
  page: Page,
  name: string,
  stylesheet = css(1, 2, 3),
) {
  await page.getByRole("button", { name: "Create theme", exact: true }).click();
  await expect(page.getByLabel("Name", { exact: true })).toHaveValue("");
  await expect(page.getByLabel("Style (CSS)", { exact: true })).toHaveValue("");
  await page.getByLabel("Name", { exact: true }).fill(name);
  await page.getByLabel("Style (CSS)", { exact: true }).fill(stylesheet);
}
async function preview(page: Page) {
  const pending = response(page, "preview");
  await page.getByRole("button", { name: "Preview", exact: true }).click();
  const result = await pending;
  expect(result.status()).toBe(200);
  return page.getByTitle("Isolated theme preview", { exact: true });
}
async function publicThemeIdentity(page: Page) {
  return {
    theme: await page.locator(".j-root").getAttribute("data-theme"),
    stylesheet: await page
      .locator("link[data-jaunder-theme-stylesheet]")
      .evaluateAll((links) => links.map((link) => link.getAttribute("href"))),
  };
}
async function expectPreviewStructure(page: Page, frame: Locator) {
  const parent = page.locator(".j-root");
  const before = await parent.evaluate((node) => {
    const s = getComputedStyle(node);
    return [s.backgroundColor, s.color, s.fontFamily];
  });
  const content = frame.contentFrame();
  const base = content.locator("link[data-jaunder-application-stylesheet]");
  await expect(frame).toHaveAttribute("sandbox", "");
  await expect(base).toHaveAttribute(
    "href",
    (await page
      .locator("head link[data-jaunder-application-stylesheet]")
      .getAttribute("href"))!,
  );
  await expect
    .poll(() =>
      base.evaluate((node) => Boolean((node as HTMLLinkElement).sheet)),
    )
    .toBe(true);
  await expect(content.locator(".j-shell")).toHaveCSS("display", "flex");
  await expect(content.locator(".j-root")).toHaveCSS(
    "box-sizing",
    "border-box",
  );
  const evidence = await content.locator(".j-shell").evaluate(async (node) => {
    await document.fonts.ready;
    let parentBlocked = false;
    try {
      void window.parent.document.body;
    } catch {
      parentBlocked = true;
    }
    const s = getComputedStyle(node);
    return {
      parentBlocked,
      background: s.backgroundColor,
      color: s.color,
      font: s.fontFamily,
      styles: Array.from(document.head.children, (child) => child.tagName),
    };
  });
  expect(evidence.parentBlocked).toBe(true);
  expect(evidence.styles.indexOf("LINK")).toBeLessThan(
    evidence.styles.indexOf("STYLE"),
  );
  await test.info().attach("preview-computed-style", {
    body: JSON.stringify(evidence),
    contentType: "application/json",
  });
  expect(
    await parent.evaluate((node) => {
      const s = getComputedStyle(node);
      return [s.backgroundColor, s.color, s.fontFamily];
    }),
  ).toEqual(before);
  await expect(parent).toHaveAttribute("data-theme", "studio");
}
async function wheelToEditorBottom(page: Page) {
  const scroll = page.locator(".j-scroll");
  const box = await scroll.boundingBox();
  expect(box).not.toBeNull();
  await page.mouse.move(box!.x + box!.width - 4, box!.y + box!.height / 2);
  const pages = await scroll.evaluate((node) =>
    Math.ceil(node.scrollHeight / node.clientHeight),
  );
  for (let i = 0; i < pages; i += 1) {
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
async function copySavedTheme(page: Page, source: string) {
  const pending = page.waitForResponse(
    (r) => new URL(r.url()).pathname === "/api/themes/copy_theme",
  );
  await page
    .getByRole("button", { name: `Copy ${source}`, exact: true })
    .click();
  return pending;
}

test.describe("route disposal during Theme mutations", () => {
  for (const stage of ["copy_theme", "publish", "select"] as const) {
    test(`${stage} completion after SPA navigation neither touches disposed UI nor replays`, async ({
      page,
      browserTrace,
    }) => {
      const trace = browserTrace();
      if (trace === undefined)
        throw new Error(
          "Theme route regression requires the default page's trace capture",
        );
      await signInAsNewUser(page);
      const initialCatalog = response(page, "list"),
        initialSelection = response(page, "get_selection");
      await goto(page, "/themes");
      await (await initialCatalog).json();
      await (await initialSelection).json();
      let id: number | undefined;
      if (stage !== "copy_theme") {
        // One setup mutation, leaving the ordinary per-user permit budget intact.
        await createLocal(page, "Route lifetime");
        const refreshedCatalog = response(page, "list"),
          refreshedSelection = response(page, "get_selection");
        id = (await save(page)).id;
        await (await refreshedCatalog).json();
        await (await refreshedSelection).json();
        await expect(
          page.getByRole("button", { name: "Use Route lifetime", exact: true }),
        ).toBeVisible();
      }
      const sentinel = await page.evaluate(() => {
        const value = crypto.randomUUID();
        Object.assign(window, { themeRouteSentinel: value });
        return value;
      });
      const writes: string[] = [],
        ownerReads: string[] = [];
      page.on("request", (r) => {
        const path = new URL(r.url()).pathname;
        if (
          [THEME_ENDPOINTS.list, THEME_ENDPOINTS.get_selection].includes(path)
        )
          ownerReads.push(path);
        if (
          [
            THEME_ENDPOINTS.copy_theme,
            THEME_ENDPOINTS.publish,
            THEME_ENDPOINTS.select,
          ].includes(path)
        )
          writes.push(path);
      });
      const release = await stallServerFn(page, `themes/${stage}`);
      const pending = page.waitForResponse(
        (r) => new URL(r.url()).pathname === THEME_ENDPOINTS[stage],
      );
      try {
        if (stage === "copy_theme") {
          await page
            .getByRole("button", { name: "Copy reader", exact: true })
            .click();
          await expect(status(page)).toContainText("Copying saved theme");
        } else {
          await page
            .getByRole("button", { name: "Use Route lifetime", exact: true })
            .click();
          await expect(status(page)).toContainText(
            stage === "publish"
              ? "Getting your theme ready"
              : "Applying your theme",
          );
        }
        await navigateInApp(
          page,
          () => page.locator('a[href="/posts/new"]').click(),
          {
            url: "/posts/new",
            ready: ".j-compose-grid",
          },
        );
      } finally {
        release();
      }
      const result = await pending;
      expect(result.status()).toBe(200);
      confirmedMutation(
        (await result.json()) as MutationOutcome<unknown>,
        `themes::${stage}`,
      );
      expect(await result.finished()).toBeNull();
      // Let the real fetch's Wasm continuation and its queued rendering work settle.
      await page.evaluate(
        () =>
          new Promise<void>((resolve) =>
            requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
          ),
      );
      expect(
        await page.evaluate(() => Reflect.get(window, "themeRouteSentinel")),
      ).toBe(sentinel);
      expect(
        trace.browserDiagnostics.filter(
          (record) => record.kind === "pageerror",
        ),
      ).toEqual([]);
      expect(ownerReads).toEqual([]);
      expect(writes).toEqual(
        stage === "copy_theme"
          ? [THEME_ENDPOINTS.copy_theme]
          : stage === "publish"
            ? [THEME_ENDPOINTS.publish]
            : [THEME_ENDPOINTS.publish, THEME_ENDPOINTS.select],
      );
      const catalog = await page.request.post(
        `${BASE_URL}${THEME_ENDPOINTS.list}`,
        { form: { scope: "author" } },
      );
      expect(catalog.status()).toBe(200);
      const entries = await catalog.json();
      expect(entries).toHaveLength(1);
      expect(entries[0].published).toBe(stage !== "copy_theme");
      const selection = await page.request.post(
        `${BASE_URL}${THEME_ENDPOINTS.get_selection}`,
        { form: { scope: "author" } },
      );
      expect(selection.status()).toBe(200);
      expect(await selection.json()).toEqual(
        stage === "select" ? { kind: "custom", value: id } : null,
      );
    });
  }
});

test("theme tiles separate explicit Use from nonactivating Edit and Copy", async ({
  page,
}) => {
  await signInAsNewUser(page);
  await goto(page, "/themes");
  const writes: string[] = [];
  page.on("request", (r) => {
    const p = new URL(r.url()).pathname;
    if ([THEME_ENDPOINTS.publish, THEME_ENDPOINTS.select].includes(p))
      writes.push(p);
  });
  const edit = page.getByRole("button", {
    name: "Preview reader",
    exact: true,
  });
  await edit.focus();
  await page.keyboard.press("Enter");
  await expect(edit).toHaveAttribute("aria-expanded", "true");
  await expect(
    page.getByRole("button", { name: "Use reader", exact: true }),
  ).toHaveAttribute("aria-pressed", "false");
  await expect(page.getByRole("dialog", { name: "Copy theme" })).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Copy Follow site theme", exact: true }),
  ).toHaveCount(0);
  for (const label of ["Preview reader", "Copy reader"]) {
    expect(
      await page
        .getByRole("button", { name: label, exact: true })
        .evaluate((n) => n.parentElement?.closest("button")),
    ).toBeNull();
  }
  expect(writes).toEqual([]);
  const applied = response(page, "select");
  await page.getByRole("button", { name: "Use reader", exact: true }).focus();
  await page.keyboard.press("Enter");
  expect((await applied).status()).toBe(200);
  await expect(page.locator('[data-theme-card="reader"]')).toHaveAttribute(
    "data-current",
    "true",
  );
  await expect(
    page.getByRole("button", { name: "Use reader", exact: true }),
  ).toHaveAttribute("aria-pressed", "true");
  expect(writes).toEqual([THEME_ENDPOINTS.select]);
});

test("bundled and saved custom Copy allocate private unused destinations without changing the source", async ({
  page,
}) => {
  await signInAsNewUser(page);
  await goto(page, "/themes");
  const writes: string[] = [];
  page.on("request", (r) => {
    const p = new URL(r.url()).pathname;
    if (
      [
        "/api/themes/copy_theme",
        THEME_ENDPOINTS.publish,
        THEME_ENDPOINTS.select,
      ].includes(p)
    )
      writes.push(p);
  });
  const first = await copySavedTheme(page, "reader");
  expect(first.status()).toBe(200);
  const source = confirmedMutation(
    (await first.json()) as MutationOutcome<{
      id: number;
      name: string;
      published: boolean;
    }>,
    "copy reader",
  );
  expect(source).toMatchObject({ name: "Reader 2", published: false });
  await expect(
    page.getByRole("button", { name: "Edit Reader 2", exact: true }),
  ).toHaveAttribute("aria-expanded", "true");
  const read = async (id: number) => {
    const r = await page.request.post(`${BASE_URL}/api/themes/get_draft`, {
      form: { scope: "author", theme_id: String(id) },
    });
    expect(r.status()).toBe(200);
    return r.json();
  };
  const saved = await read(source.id);
  await page
    .getByLabel("Style (CSS)", { exact: true })
    .fill("/* deliberately unsaved */");
  const second = await copySavedTheme(page, "Reader 2");
  expect(second.status()).toBe(200);
  const copied = confirmedMutation(
    (await second.json()) as MutationOutcome<{
      id: number;
      name: string;
      published: boolean;
    }>,
    "copy saved",
  );
  expect(copied).toMatchObject({ name: "Reader 2 2", published: false });
  expect(copied.id).not.toBe(source.id);
  await expect(
    page.getByRole("button", { name: "Edit Reader 2 2", exact: true }),
  ).toHaveAttribute("aria-expanded", "true");
  await expect(page.getByLabel("Style (CSS)", { exact: true })).not.toHaveValue(
    "/* deliberately unsaved */",
  );
  expect(await read(source.id)).toEqual(saved);
  expect(await read(copied.id)).toEqual(saved);
  await page
    .getByRole("button", { name: "Edit Reader 2", exact: true })
    .click();
  await expect(page.getByLabel("Style (CSS)", { exact: true })).toHaveValue(
    "/* deliberately unsaved */",
  );
  const repeated = await copySavedTheme(page, "reader");
  expect(await repeated.json()).toMatchObject({
    Confirmed: { name: "Reader 3", published: false },
  });
  await expect(current(page)).toHaveAttribute(
    "data-theme-selection",
    "inherit",
  );
  expect(writes).toEqual(Array(3).fill("/api/themes/copy_theme"));
});

for (const outcome of [
  "rejected",
  "unknown-empty",
  "unknown-created",
] as const) {
  test(`Copy ${outcome} reconciles once without replay, activation or editor retarget`, async ({
    page,
  }) => {
    await signInAsNewUser(page);
    await goto(page, "/themes");
    await page
      .getByRole("button", { name: "Preview studio", exact: true })
      .click();
    let copies = 0;
    const writes: string[] = [],
      reads: string[] = [];
    await page.route("**/api/themes/copy_theme", async (route) => {
      copies += 1;
      if (outcome === "unknown-created") {
        const r = await route.fetch();
        expect(r.status()).toBe(200);
        await route.fulfill({
          response: r,
          body: JSON.stringify({ CommitIndeterminate: null }),
        });
      } else if (outcome === "rejected")
        await route.fulfill({
          status: 500,
          contentType: "application/json",
          body: JSON.stringify({
            forbidden: { message: "A saved image is not available to you" },
          }),
        });
      else
        await route.fulfill({
          contentType: "application/json",
          body: JSON.stringify({ CommitIndeterminate: null }),
        });
    });
    page.on("request", (r) => {
      const p = new URL(r.url()).pathname;
      if ([THEME_ENDPOINTS.publish, THEME_ENDPOINTS.select].includes(p))
        writes.push(p);
      if ([THEME_ENDPOINTS.list, THEME_ENDPOINTS.get_selection].includes(p))
        reads.push(p);
    });
    await copySavedTheme(page, "reader");
    await expect(status(page)).toContainText(
      outcome === "rejected" ? "Copy rejected" : "Copy outcome is uncertain",
    );
    await expect(status(page)).toHaveClass(/error/);
    await expect(status(page)).not.toHaveClass(/success/);
    await expect(
      page.getByRole("button", { name: "Preview studio", exact: true }),
    ).toHaveAttribute("aria-expanded", "true");
    await expect(
      page.getByRole("button", { name: "Edit Reader 2", exact: true }),
    ).toHaveCount(outcome === "unknown-created" ? 1 : 0);
    await expect(current(page)).toHaveAttribute(
      "data-theme-selection",
      "inherit",
    );
    expect(copies).toBe(1);
    expect(writes).toEqual([]);
    expect(reads.sort()).toEqual(
      [THEME_ENDPOINTS.get_selection, THEME_ENDPOINTS.list].sort(),
    );
  });
}

test("Copy pending captures the Site source and owner and blocks duplicate dispatch", async ({
  page,
}) => {
  await signInAs(page, "testoperator");
  await goto(page, "/themes");
  const site = page.getByRole("tab", { name: "Site", exact: true }),
    user = page.getByRole("tab", { name: "User", exact: true });
  await site.click();
  await expect(site).toHaveAttribute("aria-selected", "true");
  const release = await stallServerFn(page, "themes/copy_theme");
  const requests: URLSearchParams[] = [];
  page.on("request", (r) => {
    if (new URL(r.url()).pathname === "/api/themes/copy_theme")
      requests.push(new URLSearchParams(r.postData() ?? ""));
  });
  const copy = page.getByRole("button", { name: "Copy reader", exact: true });
  const pending = page.waitForResponse(
    (r) => new URL(r.url()).pathname === "/api/themes/copy_theme",
  );
  await copy.click();
  try {
    await expect(copy).toBeDisabled();
    await expect(user).toBeDisabled();
    for (const control of [copy, user])
      await control.evaluate((n) =>
        n.dispatchEvent(new MouseEvent("click", { bubbles: true })),
      );
    expect(requests).toHaveLength(1);
    expect(requests[0].get("scope")).toBe("site");
    expect(requests[0].get("source[kind]")).toBe("built_in");
    expect(requests[0].get("source[value]")).toBe("reader");
  } finally {
    release();
  }
  const r = await pending;
  expect(r.status()).toBe(200);
  const entry = confirmedMutation(
    (await r.json()) as MutationOutcome<{ id: number }>,
    "site copy",
  );
  await expect(status(page)).toContainText("Copied Reader");
  await user.click();
  await expect(page.locator(`[data-theme-card="${entry.id}"]`)).toHaveCount(0);
  const removed = await page.request.post(
    `${BASE_URL}${THEME_ENDPOINTS.remove}`,
    { form: { scope: "site", theme_id: String(entry.id) } },
  );
  expect(removed.status()).toBe(200);
  confirmedMutation(
    (await removed.json()) as MutationOutcome<null>,
    "copy cleanup",
  );
});

for (const theme of ["studio", "terminal", "reader"]) {
  test(`installed preview renders ${theme} without applying`, async ({
    page,
  }) => {
    await signInAsNewUser(page);
    await goto(page, "/themes");
    const writes: string[] = [];
    page.on("request", (r) => {
      const p = new URL(r.url()).pathname;
      if (
        [
          THEME_ENDPOINTS.publish,
          THEME_ENDPOINTS.select,
          THEME_ENDPOINTS.save,
        ].includes(p)
      )
        writes.push(p);
    });
    const privateStyle = await page.locator(".j-root").evaluate((n) => {
      const s = getComputedStyle(n);
      return [s.backgroundColor, s.color, s.fontFamily];
    });
    await page
      .getByRole("button", { name: `Preview ${theme}`, exact: true })
      .click();
    await page.getByRole("button", { name: "Preview", exact: true }).click();
    const frame = page.getByTitle("Saved theme preview", { exact: true });
    await expect(frame.contentFrame().locator(".j-root")).toHaveAttribute(
      "data-theme",
      theme,
    );
    await expectPreviewStructure(page, frame);
    if (theme === "reader") {
      await expect(frame.contentFrame().locator(".j-shell")).toHaveCSS(
        "background-color",
        "rgb(246, 242, 234)",
      );
      await expect(frame.contentFrame().locator(".j-shell")).toHaveCSS(
        "color",
        "rgb(42, 38, 34)",
      );
      expect(
        await frame
          .contentFrame()
          .locator(".j-shell")
          .evaluate((n) =>
            getComputedStyle(n)
              .fontFamily.split(",")
              .map((f) => f.trim().replace(/^"|"$/g, "")),
          ),
      ).toEqual(["Source Serif 4", "Charter", "Georgia", "serif"]);
    }
    expect(
      await page.locator(".j-root").evaluate((n) => {
        const s = getComputedStyle(n);
        return [s.backgroundColor, s.color, s.fontFamily];
      }),
    ).toEqual(privateStyle);
    await expect(page.getByLabel("Style (CSS)", { exact: true })).toHaveCount(
      0,
    );
    await expect(current(page)).toHaveAttribute(
      "data-theme-selection",
      "inherit",
    );
    expect(writes).toEqual([]);
    await expect(
      page.locator("link[data-jaunder-theme-stylesheet]"),
    ).toHaveCount(0);
  });
}

test("current-form preview shows unsaved fields without saving or replacing package context", async ({
  page,
}) => {
  await signInAsNewUser(page);
  const id = await publishTheme(page);
  await goto(page, "/themes");
  await page
    .getByRole("button", { name: "Edit Conformance", exact: true })
    .click();
  await page.getByLabel("Name", { exact: true }).fill("Unsaved name");
  await page.getByLabel("Style (CSS)", { exact: true }).fill(css(4, 5, 6));
  await page.getByLabel("Logo", { exact: true }).selectOption("none");
  const frame = await preview(page);
  await expectPreviewStructure(page, frame);
  await expect(
    frame.contentFrame().locator('[data-jaunder-part="main"]'),
  ).toHaveCSS("outline-color", "rgb(4, 5, 6)");
  await expect(
    frame.contentFrame().locator('[data-jaunder-part="logo"]'),
  ).toHaveCount(0);
  const r = await page.request.post(`${BASE_URL}/api/themes/get_draft`, {
    form: { scope: "author", theme_id: String(id) },
  });
  const draft = await r.json();
  expect(draft.assets).toHaveLength(3);
  expect(Buffer.from(draft.stylesheet).toString()).toContain(
    "Conformance Sans",
  );
  await expect(
    page.getByRole("button", { name: "Edit Conformance", exact: true }),
  ).toBeVisible();
  await expect(current(page)).toHaveAttribute(
    "data-theme-selection",
    "inherit",
  );
  await page.getByLabel("Style (CSS)", { exact: true }).fill(css(7, 8, 9));
  await expect(frame).toHaveCount(0);
  await expect(
    page.getByText(
      "Preview is out of date. Preview again to see your current changes.",
    ),
  ).toBeVisible();
});

test("new Preview creates no row and post-Save Preview survives new-target rekey and cached return", async ({
  page,
}) => {
  await signInAsNewUser(page);
  await goto(page, "/themes");
  const writes: string[] = [];
  page.on("request", (r) => {
    const p = new URL(r.url()).pathname;
    if (
      [
        THEME_ENDPOINTS.save,
        THEME_ENDPOINTS.publish,
        THEME_ENDPOINTS.select,
      ].includes(p)
    )
      writes.push(p);
  });
  await createLocal(page, "Meadow");
  await preview(page);
  await expect(
    page.getByRole("button", { name: "Edit Meadow", exact: true }),
  ).toHaveCount(0);
  expect(writes).toEqual([]);
  const saved = await save(page);
  expect(saved.applied).toBe(false);
  await expect(
    page.getByRole("button", { name: "Edit Meadow", exact: true }),
  ).toHaveAttribute("aria-expanded", "true");
  await page.getByLabel("Style (CSS)", { exact: true }).fill(css(4, 5, 6));
  const frame = await preview(page);
  await expect(
    frame.contentFrame().locator('[data-jaunder-part="main"]'),
  ).toHaveCSS("outline-color", "rgb(4, 5, 6)");
  await page
    .getByRole("button", { name: "Preview reader", exact: true })
    .click();
  await page.getByRole("button", { name: "Edit Meadow", exact: true }).click();
  await expect(page.getByLabel("Style (CSS)", { exact: true })).toHaveValue(
    css(4, 5, 6),
  );
  await expect(current(page)).toHaveAttribute(
    "data-theme-selection",
    "inherit",
  );
  expect(writes).toEqual([THEME_ENDPOINTS.save]);
});

test("late normal preview cannot cross custom, built-in or inheritance boundaries", async ({
  page,
}) => {
  await signInAsNewUser(page);
  await publishTheme(page);
  await goto(page, "/themes");
  await page
    .getByRole("button", { name: "Edit Conformance", exact: true })
    .click();
  const release = await stallServerFn(page, "themes/preview_editor"),
    pending = response(page, "preview");
  try {
    await page.getByRole("button", { name: "Preview", exact: true }).click();
    await expect(
      page.getByText("Preparing preview…", { exact: true }),
    ).toBeVisible();
    // Preview is not a write lock.
    await expect(page.getByLabel("Name", { exact: true })).toBeEnabled();
    await page
      .getByRole("button", { name: "Preview reader", exact: true })
      .click();
  } finally {
    release();
  }
  await pending;
  await expect(page.getByTitle("Isolated theme preview")).toHaveCount(0);
  await page.getByRole("button", { name: "Preview", exact: true }).click();
  const frame = page.getByTitle("Saved theme preview");
  await expect(frame.contentFrame().locator(".j-root")).toHaveAttribute(
    "data-theme",
    "reader",
  );
  await page
    .getByRole("button", { name: "Use site theme", exact: true })
    .click();
  await expect(frame.contentFrame().locator(".j-root")).toHaveAttribute(
    "data-theme",
    "reader",
  );
  await expect(current(page)).toHaveAttribute(
    "data-theme-selection",
    "inherit",
  );
});

test("late personal preview cannot appear in the operator Site workspace", async ({
  page,
}) => {
  await signInAs(page, "testoperator");
  await goto(page, "/themes");
  const requests: URLSearchParams[] = [];
  page.on("request", (r) => {
    if (new URL(r.url()).pathname === "/api/themes/preview_installed")
      requests.push(new URLSearchParams(r.postData() ?? ""));
  });
  const release = await stallServerFn(page, "themes/preview_installed");
  const pending = page.waitForResponse(
    (r) => new URL(r.url()).pathname === "/api/themes/preview_installed",
  );
  try {
    await page
      .getByRole("button", { name: "Preview studio", exact: true })
      .click();
    await page.getByRole("button", { name: "Preview", exact: true }).click();
    await page.getByRole("tab", { name: "Site", exact: true }).click();
  } finally {
    release();
  }
  await pending;
  await expect(page.getByTitle("Saved theme preview")).toHaveCount(0);
  await page
    .getByRole("button", { name: "Preview reader", exact: true })
    .click();
  await page.getByRole("button", { name: "Preview", exact: true }).click();
  await expect(
    page.getByTitle("Saved theme preview").contentFrame().locator(".j-root"),
  ).toHaveAttribute("data-theme", "reader");
  expect(requests.map((r) => r.get("scope"))).toEqual(["author", "site"]);
  expect(requests.map((r) => r.get("theme[value]"))).toEqual([
    "studio",
    "reader",
  ]);
  await expect(
    page.getByRole("tab", { name: "Site", exact: true }),
  ).toHaveAttribute("aria-selected", "true");
});

test("installed preview failure clears old output without retry or disabling choices", async ({
  page,
}) => {
  await signInAsNewUser(page);
  await goto(page, "/themes");
  await page
    .getByRole("button", { name: "Preview studio", exact: true })
    .click();
  await page.getByRole("button", { name: "Preview", exact: true }).click();
  await expect(page.getByTitle("Saved theme preview")).toBeVisible();
  let requests = 0;
  await page.route("**/api/themes/preview_installed", (route) => {
    requests += 1;
    return route.fulfill({ status: 500, body: "preview unavailable" });
  });
  await page
    .getByRole("button", { name: "Preview reader", exact: true })
    .click();
  await expect(page.getByTitle("Saved theme preview")).toHaveCount(0);
  await page.getByRole("button", { name: "Preview", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText(
    "Saved theme preview unavailable",
  );
  await expect(page.getByTitle("Saved theme preview")).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Use reader", exact: true }),
  ).toBeEnabled();
  expect(requests).toBe(1);
});

test("active complete Save advances public revision without another Use and preserves package source buffer", async ({
  page,
  tracedContext,
}) => {
  const username = await signInAsNewUser(page),
    id = await publishAndSelectTheme(page);
  await goto(page, "/themes");
  await page
    .getByRole("button", { name: "Edit Conformance", exact: true })
    .click();
  const writes: string[] = [];
  page.on("request", (r) => {
    const p = new URL(r.url()).pathname;
    if (
      [
        THEME_ENDPOINTS.save,
        THEME_ENDPOINTS.publish,
        THEME_ENDPOINTS.select,
      ].includes(p)
    )
      writes.push(p);
  });
  await page.getByLabel("Name", { exact: true }).fill("Ocean");
  await page.getByLabel("Style (CSS)", { exact: true }).fill(css(9, 8, 7));
  await page.getByLabel("Logo", { exact: true }).selectOption("none");
  await page.getByLabel("Header image", { exact: true }).selectOption("none");
  const saved = await save(page);
  expect(saved).toMatchObject({ id, name: "Ocean", applied: true });
  expect(writes).toEqual([THEME_ENDPOINTS.save]);
  await expect(current(page)).toHaveAttribute(
    "data-theme-selection",
    String(id),
  );
  await openThemeAdvanced(page);
  const packageSource = page.getByLabel("style.css", { exact: true });
  await expect(packageSource).toHaveValue(css(9, 8, 7));
  await packageSource.fill("/* shared unsaved CSS */");
  await expect(page.getByLabel("Style (CSS)", { exact: true })).toHaveValue(
    "/* shared unsaved CSS */",
  );
  await page
    .getByRole("button", { name: "Preview reader", exact: true })
    .click();
  await page.getByRole("button", { name: "Edit Ocean", exact: true }).click();
  await expect(page.getByLabel("Style (CSS)", { exact: true })).toHaveValue(
    "/* shared unsaved CSS */",
  );
  const context = await tracedContext();
  try {
    const publicPage = await context.newPage();
    await goto(publicPage, `/~${username}`);
    await expect(publicPage.locator('[data-jaunder-part="main"]')).toHaveCSS(
      "outline-color",
      "rgb(9, 8, 7)",
    );
    await expect(publicPage.locator('[data-jaunder-part="logo"]')).toHaveCount(
      0,
    );
    await expect(
      publicPage.locator('[data-jaunder-part="header-image"]'),
    ).toHaveCount(0);
  } finally {
    await context.close();
  }
});

test("inactive Save preserves old published eligibility and does not switch appearance", async ({
  page,
  tracedContext,
}) => {
  const username = await signInAsNewUser(page);
  const id = await publishTheme(page);
  await goto(page, "/themes");
  const context = await tracedContext();
  let baseline: Awaited<ReturnType<typeof publicThemeIdentity>>;
  try {
    const p = await context.newPage();
    await goto(p, `/~${username}`);
    baseline = await publicThemeIdentity(p);
  } finally {
    await context.close();
  }
  await page
    .getByRole("button", { name: "Edit Conformance", exact: true })
    .click();
  await page.getByLabel("Style (CSS)", { exact: true }).fill(css(9, 8, 7));
  const saved = await save(page);
  expect(saved.applied).toBe(false);
  const catalog = await page.request.post(
    `${BASE_URL}${THEME_ENDPOINTS.list}`,
    { form: { scope: "author" } },
  );
  expect(await catalog.json()).toContainEqual({
    id,
    name: "Conformance",
    published: true,
  });
  await expect(current(page)).toHaveAttribute(
    "data-theme-selection",
    "inherit",
  );
  const after = await tracedContext();
  try {
    const p = await after.newPage();
    await goto(p, `/~${username}`);
    expect(await publicThemeIdentity(p)).toEqual(baseline!);
  } finally {
    await after.close();
  }
});

test("the current custom tile reapplies saved expert edits using the same captured Theme ID", async ({
  page,
  tracedContext,
}) => {
  const username = await signInAsNewUser(page);
  const id = String(await publishAndSelectTheme(page));
  await goto(page, "/themes");
  await page
    .getByRole("button", { name: "Edit Conformance", exact: true })
    .click();
  await openThemeAdvanced(page);
  await page.getByLabel("style.css", { exact: true }).fill(css(7, 8, 9));
  const saved = response(page, "import_package");
  await page
    .getByRole("button", { name: "Save complete draft package", exact: true })
    .click();
  expect((await saved).status()).toBe(200);
  await expect(status(page)).toContainText("Saved");
  const writes: { path: string; form: URLSearchParams }[] = [];
  page.on("request", (r) => {
    const p = new URL(r.url()).pathname;
    if ([THEME_ENDPOINTS.publish, THEME_ENDPOINTS.select].includes(p))
      writes.push({ path: p, form: new URLSearchParams(r.postData() ?? "") });
  });
  const use = page.getByRole("button", {
    name: "Use Conformance",
    exact: true,
  });
  await expect(use).toHaveAttribute("aria-pressed", "true");
  await use.focus();
  await page.keyboard.press("Enter");
  await expect(status(page)).toContainText("Your theme is now in use");
  expect(writes.map((w) => w.path)).toEqual([
    THEME_ENDPOINTS.publish,
    THEME_ENDPOINTS.select,
  ]);
  expect(writes[0].form.get("theme_id")).toBe(id);
  expect(writes[1].form.get("selection[value]")).toBe(id);
  const context = await tracedContext();
  try {
    const p = await context.newPage();
    await goto(p, `/~${username}`);
    await expect(p.locator('[data-jaunder-part="main"]')).toHaveCSS(
      "outline-color",
      "rgb(7, 8, 9)",
    );
  } finally {
    await context.close();
  }
});

test("operator caches independent User and Site normal editors without activating on open", async ({
  page,
}) => {
  await signInAs(page, "testoperator");
  await goto(page, "/themes");
  await expect(
    page.getByRole("tablist", { name: "Theme setting" }),
  ).toBeVisible();
  const writes: string[] = [];
  page.on("request", (r) => {
    const p = new URL(r.url()).pathname;
    if (
      [
        THEME_ENDPOINTS.save,
        THEME_ENDPOINTS.publish,
        THEME_ENDPOINTS.select,
      ].includes(p)
    )
      writes.push(p);
  });
  await createLocal(page, "Personal unsaved");
  await page.getByRole("tab", { name: "Site", exact: true }).focus();
  await page.keyboard.press("Enter");
  await createLocal(page, "Site unsaved", css(4, 5, 6));
  await page.getByRole("tab", { name: "Site", exact: true }).focus();
  await page.keyboard.press("ArrowLeft");
  await expect(
    page.getByRole("tab", { name: "User", exact: true }),
  ).toBeFocused();
  await expect(page.getByLabel("Name", { exact: true })).toHaveValue(
    "Personal unsaved",
  );
  await page.getByRole("tab", { name: "User", exact: true }).press("End");
  await expect(
    page.getByRole("tab", { name: "Site", exact: true }),
  ).toBeFocused();
  await expect(page.getByLabel("Name", { exact: true })).toHaveValue(
    "Site unsaved",
  );
  await expect(page.getByLabel("Style (CSS)", { exact: true })).toHaveValue(
    css(4, 5, 6),
  );
  expect(writes).toEqual([]);
});

test("saved User and Site settings retain separate IDs and selection after owner switches", async ({
  page,
  tracedContext,
}) => {
  await withSiteThemeSelection(tracedContext, async (mutate) => {
    await signInAs(page, "testoperator");
    await goto(page, "/themes");
    const suffix = Date.now(),
      ids: number[] = [],
      writes: URLSearchParams[] = [];
    page.on("request", (r) => {
      if (new URL(r.url()).pathname === THEME_ENDPOINTS.select)
        writes.push(new URLSearchParams(r.postData() ?? ""));
    });
    for (const [tab, name] of [
      ["User", `Personal ${suffix}`],
      ["Site", `Site ${suffix}`],
    ] as const) {
      await page.getByRole("tab", { name: tab, exact: true }).click();
      await createLocal(page, name);
      const saved = await save(page);
      ids.push(saved.id);
      await mutate(async () => {
        const published = response(page, "publish"),
          selected = response(page, "select");
        await page
          .getByRole("button", { name: `Use ${name}`, exact: true })
          .click();
        for (const pending of [published, selected]) {
          const r = await pending;
          expect(r.status()).toBe(200);
          confirmedMutation(
            (await r.json()) as MutationOutcome<null>,
            "independent setting Use",
          );
        }
      });
      await expect(current(page)).toHaveAttribute(
        "data-theme-selection",
        String(saved.id),
      );
    }
    expect(ids[0]).not.toBe(ids[1]);
    expect(writes.map((w) => w.get("scope"))).toEqual(["author", "site"]);
    expect(writes.map((w) => w.get("selection[value]"))).toEqual(
      ids.map(String),
    );
    for (const [i, tab] of ["User", "Site"].entries()) {
      await page.getByRole("tab", { name: tab, exact: true }).click();
      await expect(current(page)).toHaveAttribute(
        "data-theme-selection",
        String(ids[i]),
      );
      await expect(
        page.locator(`[data-theme-card="${ids[i]}"]`),
      ).toHaveAttribute("data-current", "true");
      await expect(
        page.locator(`[data-theme-card="${ids[1 - i]}"]`),
      ).toHaveCount(0);
      await page
        .locator(`[data-theme-card="${ids[i]}"] .j-theme-catalog-item`)
        .click();
      await openThemeAdvanced(page);
      await mutate(async () => {
        const pending = response(page, "remove");
        await page
          .getByRole("button", { name: "Delete theme", exact: true })
          .click();
        const r = await pending;
        expect(r.status()).toBe(200);
        confirmedMutation(
          (await r.json()) as MutationOutcome<null>,
          "independent setting cleanup",
        );
      });
      await expect(page.locator(`[data-theme-card="${ids[i]}"]`)).toHaveCount(
        0,
      );
    }
  });
});

for (const stage of ["publish", "select"] as const) {
  for (const fault of [
    "reject",
    "indeterminate",
    "lost",
    "undecodable",
  ] as const) {
    test(`${stage} ${fault} stops or finishes chaining honestly and rereads without retry`, async ({
      page,
      tracedContext,
    }) => {
      const username = await signInAsNewUser(page);
      await importMinimalTheme(page);
      const baselineContext = await tracedContext();
      let baseline: Awaited<ReturnType<typeof publicThemeIdentity>>;
      try {
        const p = await baselineContext.newPage();
        await goto(p, `/~${username}`);
        baseline = await publicThemeIdentity(p);
      } finally {
        await baselineContext.close();
      }
      const writes: { path: string; body: string }[] = [],
        reads: string[] = [];
      page.on("request", (r) => {
        const p = new URL(r.url()).pathname;
        if ([THEME_ENDPOINTS.publish, THEME_ENDPOINTS.select].includes(p))
          writes.push({ path: p, body: r.postData() ?? "" });
        if ([THEME_ENDPOINTS.list, THEME_ENDPOINTS.get_selection].includes(p))
          reads.push(p);
      });
      await page.route(`**${THEME_ENDPOINTS[stage]}`, async (route) => {
        if (fault === "reject") {
          await route.fulfill({
            status: 500,
            contentType: "application/json",
            body: JSON.stringify({
              validation: { message: "Rejected for this test" },
            }),
          });
          return;
        }
        const r = await route.fetch();
        expect(r.status()).toBe(200);
        expect(await r.json()).toEqual({ Confirmed: null });
        if (fault === "lost") await route.abort("failed");
        else
          await route.fulfill({
            response: r,
            body:
              fault === "indeterminate"
                ? JSON.stringify({ CommitIndeterminate: null })
                : "undecodable acknowledgement",
          });
      });
      const use = page.getByRole("button", {
        name: "Use Intake fixture",
        exact: true,
      });
      await use.click();
      const s = status(page);
      await expect(s).toContainText(
        fault === "reject"
          ? stage === "publish"
            ? "Getting your theme ready was rejected"
            : "switching was rejected"
          : stage === "publish"
            ? "We could not confirm your theme is ready"
            : "switching outcome is uncertain",
      );
      await expect(s).not.toContainText("Your theme is now in use");
      await expect(s).toHaveClass(/error/);
      await expect(s).not.toHaveClass(/success/);
      await expect(s).toContainText("Available themes checked");
      await expect(s).toContainText("Current theme:");
      await expect(use).toBeEnabled();
      expect(writes.map((w) => w.path)).toEqual(
        stage === "publish"
          ? [THEME_ENDPOINTS.publish]
          : [THEME_ENDPOINTS.publish, THEME_ENDPOINTS.select],
      );
      expect(reads).toContain(THEME_ENDPOINTS.list);
      expect(reads).toContain(THEME_ENDPOINTS.get_selection);
      expect(new URLSearchParams(writes[0].body).get("scope")).toBe("author");
      if (stage === "select") {
        await expect(s).toContainText("Your updated theme is ready");
        expect(
          new URLSearchParams(writes[1].body).get("selection[value]"),
        ).toBe(new URLSearchParams(writes[0].body).get("theme_id"));
      } else
        await expect(s).toContainText(
          fault === "reject"
            ? "Fix the theme"
            : "We did not try to switch themes",
        );
      const applied = stage === "select" && fault !== "reject";
      if (!applied)
        await expect(current(page)).toHaveAttribute(
          "data-theme-selection",
          "inherit",
        );
      else
        await expect(current(page)).not.toHaveAttribute(
          "data-theme-selection",
          "inherit",
        );
      const context = await tracedContext();
      try {
        const p = await context.newPage();
        await goto(p, `/~${username}`);
        if (!applied) expect(await publicThemeIdentity(p)).toEqual(baseline!);
        else {
          await expect(p.locator(".j-root")).toHaveAttribute(
            "data-theme",
            "custom",
          );
          await expect(p.locator('[data-jaunder-part="main"]')).toHaveCSS(
            "outline-color",
            "rgb(1, 2, 3)",
          );
        }
      } finally {
        await context.close();
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
    page.on("request", (r) => {
      const p = new URL(r.url()).pathname;
      if ([THEME_ENDPOINTS.publish, THEME_ENDPOINTS.select].includes(p))
        writes.push(p);
    });
    for (const endpoint of [
      THEME_ENDPOINTS.list,
      THEME_ENDPOINTS.get_selection,
    ])
      await page.route(`**${endpoint}`, async (route) => {
        if (dispatched)
          await route.fulfill({ status: 500, body: "reread unavailable" });
        else await route.continue();
      });
    await page.route(`**${THEME_ENDPOINTS[stage]}`, async (route) => {
      const r = await route.fetch();
      expect(r.status()).toBe(200);
      expect(await r.json()).toEqual({ Confirmed: null });
      dispatched = true;
      await route.fulfill({
        response: r,
        body: JSON.stringify({ CommitIndeterminate: null }),
      });
    });
    const use = page.getByRole("button", {
      name: "Use Intake fixture",
      exact: true,
    });
    await use.click();
    await expect(status(page)).toContainText("Theme readiness is unknown");
    await expect(status(page)).toContainText("current setting is unknown");
    await expect(status(page)).toContainText("reopen Themes");
    await expect(status(page)).not.toContainText(
      "Current theme: Using the site theme",
    );
    await expect(use).toBeEnabled();
    await expect(
      page.getByRole("button", { name: "Use site theme", exact: true }),
    ).toBeDisabled();
    await expect(current(page)).toHaveAttribute("data-theme-selection", "");
    await expect(current(page)).toHaveText(
      "Current theme could not be checked",
    );
    expect(writes).toEqual(
      stage === "publish"
        ? [THEME_ENDPOINTS.publish]
        : [THEME_ENDPOINTS.publish, THEME_ENDPOINTS.select],
    );
  });
}

test("pending normal Save blocks editable fields, Use, duplicate Save and scope changes", async ({
  page,
}) => {
  await signInAs(page, "testoperator");
  await goto(page, "/themes");
  await createLocal(page, `Pending ${Date.now()}`);
  const release = await stallServerFn(page, "themes/save_editor"),
    writes: string[] = [];
  page.on("request", (r) => {
    const p = new URL(r.url()).pathname;
    if (
      [
        THEME_ENDPOINTS.save,
        THEME_ENDPOINTS.publish,
        THEME_ENDPOINTS.select,
      ].includes(p)
    )
      writes.push(p);
  });
  const pending = response(page, "save"),
    button = page.getByRole("button", { name: "Save changes", exact: true }),
    site = page.getByRole("tab", { name: "Site", exact: true }),
    use = page.getByRole("button", { name: "Use reader", exact: true });
  await button.click();
  try {
    for (const field of ["Name", "Style (CSS)", "Logo", "Header image"])
      await expect(page.getByLabel(field, { exact: true })).toBeDisabled();
    for (const control of [button, site, use]) {
      await expect(control).toBeDisabled();
      await control.evaluate((n) =>
        n.dispatchEvent(new MouseEvent("click", { bubbles: true })),
      );
    }
    await normal(page).dispatchEvent("submit");
    await expect(status(page)).toContainText("Saving changes");
    await expect(status(page)).not.toHaveClass(/success|error/);
    expect(writes).toEqual([THEME_ENDPOINTS.save]);
  } finally {
    release();
  }
  const r = await pending;
  expect(r.status()).toBe(200);
  confirmedMutation(
    (await r.json()) as MutationOutcome<unknown>,
    "pending save",
  );
  await expect(status(page)).toHaveText("Changes saved.");
  await expect(button).toBeEnabled();
  expect(writes).toEqual([THEME_ENDPOINTS.save]);
});

for (const fault of [
  "rejected",
  "indeterminate",
  "lost",
  "undecodable",
] as const) {
  test(`normal Save ${fault} never replays or adopts an uncertain new ID`, async ({
    page,
  }) => {
    await signInAsNewUser(page);
    await goto(page, "/themes");
    await createLocal(page, "Uncertain");
    let attempts = 0;
    let diagnostic = 0;
    let committed: { id: number; name: string } | undefined;
    const ownerReads: string[] = [];
    page.on("request", (r) => {
      const path = new URL(r.url()).pathname;
      if (path === "/api/themes/get_draft") diagnostic += 1;
      if ([THEME_ENDPOINTS.list, THEME_ENDPOINTS.get_selection].includes(path))
        ownerReads.push(path);
    });
    await page.route(`**${THEME_ENDPOINTS.save}`, async (route) => {
      attempts += 1;
      if (fault === "rejected") {
        await route.fulfill({
          status: 500,
          contentType: "application/json",
          body: JSON.stringify({
            validation: { message: "Explicit Save rejection" },
          }),
        });
        return;
      }
      // Commit once for real, then replace only its acknowledgement.
      const r = await route.fetch();
      expect(r.status()).toBe(200);
      const body = await r.json();
      expect(body.Confirmed).toBeDefined();
      committed = body.Confirmed;
      if (fault === "lost") await route.abort("failed");
      else
        await route.fulfill({
          response: r,
          body:
            fault === "indeterminate"
              ? JSON.stringify({ CommitIndeterminate: body.Confirmed })
              : "undecodable",
        });
    });
    await page
      .getByRole("button", { name: "Save changes", exact: true })
      .click();
    await expect(status(page)).toContainText(
      fault === "rejected" ? "Save was rejected" : "could not confirm the save",
    );
    await expect(status(page)).not.toContainText("Changes were not saved");
    if (fault === "lost" || fault === "undecodable")
      await expect(status(page)).toContainText("may already have been saved");
    await expect(status(page)).not.toHaveClass(/success/);
    await expect(page.getByLabel("Name", { exact: true })).toHaveValue(
      "Uncertain",
    );
    await expect(
      page.getByText("Unsaved changes", { exact: true }),
    ).toBeVisible();
    await expect
      .poll(() => ownerReads.slice().sort())
      .toEqual([THEME_ENDPOINTS.list, THEME_ENDPOINTS.get_selection].sort());
    const catalog = await page.request.post(
      `${BASE_URL}${THEME_ENDPOINTS.list}`,
      {
        form: { scope: "author" },
      },
    );
    expect(catalog.status()).toBe(200);
    if (fault === "rejected") {
      expect(committed).toBeUndefined();
      expect(await catalog.json()).toEqual([]);
      await expect(
        page.getByRole("button", { name: "Edit Uncertain", exact: true }),
      ).toHaveCount(0);
    } else {
      expect(committed).toBeDefined();
      expect(await catalog.json()).toEqual([
        { id: committed!.id, name: "Uncertain", published: false },
      ]);
      await expect(
        page.getByRole("button", { name: "Edit Uncertain", exact: true }),
      ).toHaveAttribute("aria-expanded", "false");
    }
    await expect(current(page)).toHaveAttribute(
      "data-theme-selection",
      "inherit",
    );
    expect(attempts).toBe(1);
    // Without an acknowledged ID, reconcile owner reads only; never invent a target.
    expect(diagnostic).toBe(fault === "indeterminate" ? 1 : 0);
  });
}

for (const operation of ["preview", "export"] as const) {
  test(`${operation} failure is live and does not publish`, async ({
    page,
  }) => {
    await signInAsNewUser(page);
    await importMinimalTheme(page);
    if (operation === "export") await openThemeAdvanced(page);
    await failServerFn(
      page,
      operation === "preview" ? "themes/preview_editor" : "themes/export",
    );
    const writes: string[] = [];
    page.on("request", (r) => {
      const p = new URL(r.url()).pathname;
      if (
        [
          THEME_ENDPOINTS.publish,
          THEME_ENDPOINTS.select,
          THEME_ENDPOINTS.save,
        ].includes(p)
      )
        writes.push(p);
    });
    await page
      .getByRole("button", {
        name: operation === "preview" ? "Preview" : "Export ZIP",
        exact: true,
      })
      .click();
    const feedback =
      operation === "preview" ? page.getByRole("alert") : status(page);
    await expect(feedback).toContainText(
      operation === "preview" ? "Preview failed" : "Export failed",
    );
    if (operation === "export")
      await expect(feedback).toContainText("Reopen Themes");
    await expect(
      page.getByRole("button", { name: "Use Intake fixture", exact: true }),
    ).toBeEnabled();
    expect(writes).toEqual([]);
  });
}

test("invalid package assets gives error feedback outside expert tools", async ({
  page,
}) => {
  await signInAsNewUser(page);
  await importMinimalTheme(page);
  await expect(status(page)).toContainText("Uploaded Intake fixture");
  await expect(status(page)).toHaveClass(/success/);
  await openThemeAdvanced(page);
  await page.getByLabel("Package assets JSON").fill("not JSON");
  await page
    .getByRole("button", { name: "Save complete draft package", exact: true })
    .click();
  await expect(status(page)).toContainText("Package assets must be valid JSON");
  await expect(status(page)).toHaveClass(/error/);
  await expect(status(page)).not.toHaveClass(/success/);
  expect(
    await status(page).evaluate((n) => Boolean(n.closest("details"))),
  ).toBe(false);
});

test("already-selected expert publication advances revision even when applying is rejected", async ({
  page,
  tracedContext,
}) => {
  const username = await signInAsNewUser(page),
    selected = String(await publishAndSelectTheme(page));
  await goto(page, "/themes");
  await page
    .getByRole("button", { name: "Edit Conformance", exact: true })
    .click();
  await openThemeAdvanced(page);
  await page.getByLabel("style.css", { exact: true }).fill(css(4, 5, 6));
  const pending = response(page, "import_package");
  await page
    .getByRole("button", { name: "Save complete draft package", exact: true })
    .click();
  expect((await pending).status()).toBe(200);
  await expect(status(page)).toContainText("Saved");
  await page.route(`**${THEME_ENDPOINTS.select}`, (route) =>
    route.fulfill({
      status: 500,
      contentType: "application/json",
      body: JSON.stringify({
        validation: { message: "Applying rejected for this test" },
      }),
    }),
  );
  await page
    .getByRole("button", { name: "Use Conformance", exact: true })
    .click();
  await expect(status(page)).toContainText(
    "Your updated theme is ready, but switching was rejected",
  );
  await expect(status(page)).toContainText(
    "already in use, the new version is already live",
  );
  await expect(current(page)).toHaveAttribute("data-theme-selection", selected);
  const context = await tracedContext();
  try {
    const p = await context.newPage();
    await goto(p, `/~${username}`);
    await expect(p.locator('[data-jaunder-part="main"]')).toHaveCSS(
      "outline-color",
      "rgb(4, 5, 6)",
    );
  } finally {
    await context.close();
  }
});

test("ZIP import opens an unselected editor and explicit Use publishes then applies it", async ({
  page,
  tracedContext,
}) => {
  const username = await signInAsNewUser(page),
    archive = await minimalThemeZip(page);
  await goto(page, "/themes");
  await expect(
    page.getByRole("heading", { level: 1, name: "Themes", exact: true }),
  ).toBeVisible();
  await expect(page.getByLabel("Theme name (optional)")).toHaveCount(0);
  const release = await stallServerFn(page, "themes/import_zip");
  try {
    await page.getByLabel("Theme ZIP file").setInputFiles({
      name: "filename-disagrees.zip",
      mimeType: "application/zip",
      buffer: archive,
    });
    await expect(status(page)).toContainText("Uploading");
    await expect(status(page)).not.toHaveClass(/success|error/);
  } finally {
    release();
  }
  const edit = page.getByRole("button", {
    name: "Edit Intake fixture",
    exact: true,
  });
  await expect(edit).toHaveAttribute("aria-expanded", "true");
  await expect(current(page)).toHaveAttribute(
    "data-theme-selection",
    "inherit",
  );
  await expect(page.getByLabel("theme.json")).toBeHidden();
  const releasePublish = await stallServerFn(page, "themes/publish"),
    releaseSelect = await stallServerFn(page, "themes/select");
  const writes: string[] = [];
  page.on("request", (r) => {
    const p = new URL(r.url()).pathname;
    if ([THEME_ENDPOINTS.publish, THEME_ENDPOINTS.select].includes(p))
      writes.push(p);
  });
  const use = page.getByRole("button", {
    name: "Use Intake fixture",
    exact: true,
  });
  try {
    await use.click();
    await expect(status(page)).toContainText("Getting your theme ready");
    await expect(use).toBeDisabled();
    await expect(edit).toBeDisabled();
    await expect(
      page.getByRole("button", { name: "Use site theme", exact: true }),
    ).toBeDisabled();
    expect(writes).toEqual([THEME_ENDPOINTS.publish]);
    releasePublish();
    await expect(status(page)).toContainText("Applying your theme");
    await expect(status(page)).not.toHaveClass(/success|error/);
  } finally {
    releasePublish();
    releaseSelect();
  }
  await expect(status(page)).toContainText("Your theme is now in use");
  await expect(status(page)).toHaveClass(/success/);
  expect(writes).toEqual([THEME_ENDPOINTS.publish, THEME_ENDPOINTS.select]);
  await expect(use).toHaveAttribute("aria-pressed", "true");
  const context = await tracedContext();
  try {
    const p = await context.newPage();
    await goto(p, `/~${username}`);
    await expect(p.locator('[data-jaunder-part="main"]')).toHaveCSS(
      "outline-color",
      "rgb(1, 2, 3)",
    );
  } finally {
    await context.close();
  }
});

test("expert disclosures retain inputs, hide collapsed controls and preserve normal fields", async ({
  page,
}) => {
  await signInAsNewUser(page);
  await importMinimalTheme(page);
  await openThemeAdvanced(page);
  const fields = [
    [page.getByLabel("theme.json", { exact: true }), '{"unsaved":"manifest"}'],
    [page.getByLabel("style.css", { exact: true }), "/* shared CSS */"],
    [page.getByLabel("Package assets JSON"), "[]"],
  ] as const;
  for (const [f, v] of fields) await f.fill(v);
  const tools = page.locator("summary").filter({ hasText: /^Theme tools$/ });
  await tools.click();
  for (const [f] of fields) {
    await expect(f).toBeHidden();
    expect(
      await f.evaluate((n) => {
        (n as HTMLElement).focus();
        return document.activeElement === n;
      }),
    ).toBe(false);
  }
  await expect(page.getByLabel("Style (CSS)", { exact: true })).toHaveValue(
    "/* shared CSS */",
  );
  await tools.focus();
  await page.keyboard.press("Tab");
  expect(
    await page.evaluate(() =>
      Boolean(document.activeElement?.closest("details:not([open])")),
    ),
  ).toBe(false);
  await tools.click();
  for (const [f, v] of fields) await expect(f).toHaveValue(v);
});

test("upload omits catalog override and uses package name", async ({
  page,
}) => {
  await signInAsNewUser(page);
  const archive = await minimalThemeZip(page);
  await goto(page, "/themes");
  await page.evaluate(() => {
    const original = window.fetch;
    window.fetch = async function (input, init) {
      const url = input instanceof Request ? input.url : String(input);
      if (url.includes("/api/themes/import_zip")) {
        const form =
          input instanceof Request
            ? await input.clone().formData()
            : init?.body;
        if (!(form instanceof FormData))
          throw new Error("Expected native multipart");
        document.documentElement.dataset.uploadFields = Array.from(
          form.keys(),
        ).join(",");
      }
      return original.call(this, input, init);
    };
  });
  await page.getByLabel("Theme ZIP file").setInputFiles({
    name: "different-filename.zip",
    mimeType: "application/zip",
    buffer: archive,
  });
  await expect(
    page.getByRole("button", { name: "Edit Intake fixture", exact: true }),
  ).toBeVisible();
  await expect(page.locator("html")).toHaveAttribute(
    "data-upload-fields",
    "scope,archive",
  );
  await expect(current(page)).toHaveAttribute(
    "data-theme-selection",
    "inherit",
  );
});

test("cancelled or malformed ZIP has no false import or selection", async ({
  page,
}) => {
  await signInAsNewUser(page);
  await goto(page, "/themes");
  let imports = 0;
  page.on("request", (r) => {
    if (new URL(r.url()).pathname === THEME_ENDPOINTS.import_zip) imports += 1;
  });
  const picker = page.waitForEvent("filechooser");
  await page
    .getByRole("button", { name: "Upload theme ZIP", exact: true })
    .click();
  await picker;
  await page.getByLabel("Theme ZIP file").dispatchEvent("cancel");
  expect(imports).toBe(0);
  await page.getByLabel("Theme ZIP file").setInputFiles({
    name: "not-a-package.zip",
    mimeType: "application/zip",
    buffer: Buffer.from("not a ZIP package"),
  });
  await expect(status(page)).toContainText("Upload rejected");
  await expect(status(page)).toContainText("Choose a valid theme ZIP file");
  await expect(status(page)).toHaveAttribute("aria-live", "polite");
  await expect(status(page)).not.toContainText("Uploaded");
  expect(imports).toBe(1);
  await expect(page.locator(".j-theme-catalog-item")).toHaveCount(0);
  await expect(current(page)).toHaveAttribute(
    "data-theme-selection",
    "inherit",
  );
});

for (const manifestName of ["Studio", "x".repeat(101)]) {
  test(`automatic package name ${manifestName.length === 101 ? "too long rejects" : "reserved receives suffix"}; explicit names stay strict`, async ({
    page,
  }) => {
    await signInAsNewUser(page);
    const archive = await minimalThemeZip(page, manifestName);
    await goto(page, "/themes");
    await page.getByLabel("Theme ZIP file").setInputFiles({
      name: "not-the-label.zip",
      mimeType: "application/zip",
      buffer: archive,
    });
    if (manifestName === "Studio") {
      await expect(
        page.getByRole("button", { name: "Edit Studio 2", exact: true }),
      ).toBeVisible();
      await expect(status(page)).toContainText("Uploaded Studio 2");
    } else {
      await expect(status(page)).toContainText("Upload rejected");
      await expect(page.locator(".j-theme-catalog-item")).toHaveCount(0);
    }
    const upload = async (name: string) =>
      page.request.post(`${BASE_URL}${THEME_ENDPOINTS.import_zip}`, {
        multipart: {
          scope: "author",
          name,
          archive: {
            name: "fixture.zip",
            mimeType: "application/zip",
            buffer: archive,
          },
        },
      });
    const rejected = await upload(manifestName);
    expect(rejected.ok()).toBe(false);
    expect(await rejected.json()).toMatchObject({
      validation: {
        message: expect.stringMatching(
          manifestName === "Studio" ? /built-in/ : /100/,
        ),
      },
    });
    const recovered = await upload("Recovered label");
    expect(recovered.status()).toBe(200);
    expect(
      confirmedMutation(
        (await recovered.json()) as MutationOutcome<{
          name: string;
          published: boolean;
        }>,
        "explicit upload",
      ),
    ).toMatchObject({ name: "Recovered label", published: false });
    await expect(current(page)).toHaveAttribute(
      "data-theme-selection",
      "inherit",
    );
  });
}

test("repeated upload receives numbered name while explicit duplicates reject", async ({
  page,
}) => {
  await signInAsNewUser(page);
  const archive = await minimalThemeZip(page);
  await goto(page, "/themes");
  for (const name of ["Intake fixture", "Intake fixture 2"]) {
    await page.getByLabel("Theme ZIP file").setInputFiles({
      name: "same.zip",
      mimeType: "application/zip",
      buffer: archive,
    });
    await expect(
      page.getByRole("button", { name: `Edit ${name}`, exact: true }),
    ).toHaveAttribute("aria-expanded", "true");
    await expect(status(page)).toContainText(`Uploaded ${name}`);
  }
  const rejected = await page.request.post(
    `${BASE_URL}${THEME_ENDPOINTS.import_zip}`,
    {
      multipart: {
        scope: "author",
        name: "Intake fixture",
        archive: {
          name: "fixture.zip",
          mimeType: "application/zip",
          buffer: archive,
        },
      },
    },
  );
  expect(rejected.ok()).toBe(false);
  await expect(page.locator(".j-theme-catalog-item")).toHaveCount(2);
  await expect(current(page)).toHaveAttribute(
    "data-theme-selection",
    "inherit",
  );
});

for (const viewport of [
  { width: 320, height: 844 },
  { width: 390, height: 844 },
  { width: 1440, height: 900 },
]) {
  test(`Option A toolbar geometry and keyboard-wheel access at ${viewport.width}`, async ({
    page,
  }) => {
    await page.setViewportSize(viewport);
    await page.emulateMedia({ colorScheme: "light", reducedMotion: "reduce" });
    await signInAsNewUser(page);
    await importMinimalTheme(page);
    expect(await page.evaluate(() => devicePixelRatio)).toBe(1);
    const heading = await page
        .getByRole("heading", { name: "Available themes", exact: true })
        .boundingBox(),
      create = await page
        .getByRole("button", { name: "Create theme", exact: true })
        .boundingBox(),
      upload = await page
        .getByRole("button", { name: "Upload theme ZIP", exact: true })
        .boundingBox();
    expect(heading && create && upload).toBeTruthy();
    expect(create!.height).toBeGreaterThanOrEqual(44);
    expect(upload!.height).toBeGreaterThanOrEqual(44);
    if (viewport.width < 760) {
      expect(create!.y).toBeGreaterThanOrEqual(heading!.y + heading!.height);
      expect(upload!.y).toBeGreaterThanOrEqual(create!.y + create!.height);
      expect(create!.width).toBeCloseTo(upload!.width, 0);
    } else {
      expect(create!.x).toBeGreaterThan(heading!.x + heading!.width);
      expect(upload!.x).toBeGreaterThan(create!.x);
    }
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    const tools = page.locator("summary").filter({ hasText: /^Theme tools$/ }),
      manifest = page.getByLabel("theme.json", { exact: true });
    await expect(manifest).toBeHidden();
    await tools.focus();
    await page.keyboard.press("Enter");
    await expect(tools).toBeFocused();
    const packageSummary = page
      .locator("summary")
      .filter({ hasText: /^Theme Package \(developer tools\)$/ });
    await packageSummary.focus();
    await page.keyboard.press("Space");
    await manifest.fill('{"unsaved":"reachable"}');
    await packageSummary.focus();
    await page.keyboard.press("Enter");
    await expect(manifest).toBeHidden();
    await page.keyboard.press("Enter");
    await expect(manifest).toHaveValue('{"unsaved":"reachable"}');
    await openThemeAdvanced(page);
    await expectAccessible(page);
    await wheelToEditorBottom(page);
    const bottom = page.getByRole("button", {
      name: "Delete theme",
      exact: true,
    });
    await expect(bottom).toBeInViewport({ ratio: 1 });
    expect(
      await page.locator(".j-scroll").evaluate((n) => n.scrollTop),
    ).toBeGreaterThan(0);
    await tools.focus();
    let reached = false;
    for (let i = 0; i < 60; i += 1) {
      await page.keyboard.press("Tab");
      if (await bottom.evaluate((n) => document.activeElement === n)) {
        reached = true;
        break;
      }
      expect(
        await page.evaluate(() =>
          Boolean(document.activeElement?.closest("details:not([open])")),
        ),
      ).toBe(false);
    }
    expect(reached).toBe(true);
    await expect(bottom).toBeFocused();
    await expect(bottom).toBeInViewport({ ratio: 1 });
    await tools.focus();
    await page.keyboard.press("Enter");
    const use = page.getByRole("button", {
        name: "Use Intake fixture",
        exact: true,
      }),
      release = await stallServerFn(page, "themes/publish");
    try {
      await use.focus();
      await page.keyboard.press("Enter");
      await expect(status(page)).toContainText("Getting your theme ready");
      await expect(status(page)).toHaveAttribute("aria-live", "polite");
      expect(
        await status(page).evaluate((n) => Boolean(n.closest("details"))),
      ).toBe(false);
    } finally {
      release();
    }
    await expect(status(page)).toContainText("Your theme is now in use");
    await expect(use).toBeEnabled();
    await expect(status(page)).toBeInViewport();
    await page.route(`**${THEME_ENDPOINTS.publish}`, (route) =>
      route.fulfill({
        status: 500,
        contentType: "application/json",
        body: JSON.stringify({ validation: { message: "Draft needs repair" } }),
      }),
    );
    await use.click();
    await expect(status(page)).toContainText(
      "Getting your theme ready was rejected",
    );
    await expect(status(page)).toContainText("Fix the theme");
    await expect(status(page)).toHaveClass(/error/);
    await page.mouse.move(viewport.width - 40, 350);
    await page.mouse.wheel(0, 20000);
    await expect(status(page)).toBeInViewport({ ratio: 1 });
  });
}

test("theme management mounts as Studio with real authoring help and no public stylesheet", async ({
  page,
}) => {
  await signInAsNewUser(page);
  await goto(page, "/themes");
  await openThemeAdvanced(page);
  const guide = page.getByRole("link", {
    name: "Read the theme repository guide",
    exact: true,
  });
  await expect(guide).toHaveAttribute(
    "href",
    "https://github.com/jaunder-org/jaunder/blob/main/docs/themes.md",
  );
  await expect(guide).toHaveAttribute("target", "_blank");
  await expect(page.locator(".j-loading")).toHaveCount(0);
  await expect(page.locator(".j-theme-catalog-item")).toHaveCount(0);
  await expect(page.locator(".j-root")).toHaveAttribute("data-theme", "studio");
  await expect(page.locator("link[data-jaunder-theme-stylesheet]")).toHaveCount(
    0,
  );
  await expect(page.getByText("No saved themes.", { exact: true })).toHaveCount(
    0,
  );
  await expectAccessible(page);
});

test("theme catalog loading does not offer empty-catalog filler", async ({
  page,
}) => {
  await signInAsNewUser(page);
  const release = await stallServerFn(page, "themes/list");
  try {
    await goto(page, "/themes");
    await expect(page.getByText("Loading available themes…")).toBeVisible();
    await expect(page.locator(".j-theme-catalog-empty")).toHaveCount(0);
  } finally {
    release();
  }
  await expect(page.getByText("Loading available themes…")).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Upload theme ZIP", exact: true }),
  ).toBeEnabled();
  await expect(
    page.getByRole("button", { name: "Copy reader", exact: true }),
  ).toBeEnabled();
});
test("theme catalog failure does not masquerade as empty", async ({ page }) => {
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
  await createLocal(
    page,
    "Rejected external",
    ".j-post { background-image: url(https://example.invalid/image.png); }",
  );
  await page.getByRole("button", { name: "Save changes", exact: true }).click();
  await expect(status(page)).toContainText("invalid theme package");
  await expect(
    page.getByRole("button", { name: "Edit Rejected external", exact: true }),
  ).toHaveCount(0);
});

test("staged Media survives pagination, a pending read, unavailable page and recovery without hiding the loaded form", async ({
  page,
}) => {
  await signInAsNewUser(page);
  const item = (n: number) => {
    const sha256 = n.toString(16).padStart(64, "0");
    const filename =
      n === 1 ? "logo-A.png" : n === 51 ? "header-B.png" : `filler-${n}.png`;
    return {
      sha256,
      filename,
      source: "upload",
      content_type: "image/png",
      size_bytes: 100,
      url: `/media/upload/${sha256.slice(0, 2)}/${sha256.slice(2, 4)}/${sha256}/${filename}`,
      created_at: "2026-10-20T00:00:00Z",
    };
  };
  const token = (n: number) =>
    `staged-media:upload:${item(n).sha256}:${item(n).filename}`;
  let release!: () => void;
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  let loading!: () => void;
  const started = new Promise<void>((resolve) => {
    loading = resolve;
  });
  // Synthetic READS exercise the real DOM graph; these choices are never submitted.
  await page.route("**/api/media/list_mine", async (route) => {
    const offset = Number(
      new URLSearchParams(route.request().postData() ?? "").get("offset") ??
        "0",
    );
    if (offset === 100) {
      loading();
      await gate;
      await route.fulfill({
        status: 500,
        contentType: "application/json",
        body: JSON.stringify({
          validation: { message: "Media page unavailable for this test" },
        }),
      });
    } else
      await route.fulfill({
        contentType: "application/json",
        body: JSON.stringify(
          Array.from({ length: 50 }, (_, i) => item(offset + i + 1)),
        ),
      });
  });
  const writes: string[] = [];
  page.on("request", (r) => {
    const p = new URL(r.url()).pathname;
    if (
      [
        THEME_ENDPOINTS.save,
        THEME_ENDPOINTS.publish,
        THEME_ENDPOINTS.select,
      ].includes(p)
    )
      writes.push(p);
  });
  try {
    await goto(page, "/themes");
    await createLocal(page, "Unsubmitted image choices");
    const logo = page.getByLabel("Logo", { exact: true }),
      header = page.getByLabel("Header image", { exact: true });
    const next = normal(page).getByRole("button", {
        name: "Next images",
        exact: true,
      }),
      previous = normal(page).getByRole("button", {
        name: "Previous images",
        exact: true,
      });
    const check = async () => {
      await expect(logo).toHaveValue(token(1));
      await expect(header).toHaveValue(token(51));
      for (const [field, n] of [
        [logo, 1],
        [header, 51],
      ] as const) {
        await expect(field.locator(`option[value="${token(n)}"]`)).toHaveCount(
          1,
        );
        expect(
          await field.evaluate((node: HTMLSelectElement) =>
            Array.from(node.selectedOptions, (o) => o.value),
          ),
        ).toEqual([token(n)]);
      }
      await expect(page.getByLabel("Name", { exact: true })).toHaveValue(
        "Unsubmitted image choices",
      );
      await expect(page.getByLabel("Style (CSS)", { exact: true })).toHaveValue(
        css(1, 2, 3),
      );
    };
    await logo.selectOption(token(1));
    await next.click();
    await header.selectOption(token(51));
    await check();
    await previous.click();
    await expect(
      logo.locator("option").filter({ hasText: "Media: filler-2.png" }),
    ).toHaveCount(1);
    await check();
    await next.click();
    await expect(
      header.locator("option").filter({ hasText: "Media: filler-52.png" }),
    ).toHaveCount(1);
    await check();
    await next.click();
    await started;
    await check();
    release();
    await expect(
      page
        .getByText("Media page unavailable for this test", { exact: false })
        .first(),
    ).toBeVisible();
    await check();
    await previous.click();
    await expect(
      header.locator("option").filter({ hasText: "Media: filler-52.png" }),
    ).toHaveCount(1);
    await check();
    expect(writes).toEqual([]);
  } finally {
    release();
    await page.unroute("**/api/media/list_mine");
  }
});

// ORIGINAL inputs pass through the real image preparation worker. Public bindings
// are compared against its returned prepared identities, never original hashes.
test("author completes package, Media rotation, preview, export and private reimport lifecycle", async ({
  page,
  tracedContext,
}) => {
  const username = await signInAsNewUser(page);
  const original = readFileSync(
      resolve(imageFixtureDirectory, "png-original.png"),
    ),
    color = readFileSync(
      resolve(imageFixtureDirectory, "png-color-original.png"),
    );
  const logo = await uploadMedia(page, "blog-logo.png", original, "image/png"),
    headerOne = await uploadMedia(
      page,
      "header-one.png",
      original,
      "image/png",
    ),
    headerTwo = await uploadMedia(page, "header-two.png", color, "image/png");
  const bytes = async (url: string) => {
    const r = await page.request.get(BASE_URL + url);
    expect(r.status()).toBe(200);
    const b = await r.body();
    expect(url).toContain(createHash("sha256").update(b).digest("hex"));
    return b;
  };
  const logoBytes = await bytes(logo.url),
    oneBytes = await bytes(headerOne.url),
    twoBytes = await bytes(headerTwo.url);
  expect(createHash("sha256").update(oneBytes).digest("hex")).not.toBe(
    createHash("sha256").update(twoBytes).digest("hex"),
  );
  for (let i = 0; i < 50; i += 1)
    await uploadMedia(
      page,
      `newer-${i.toString().padStart(2, "0")}.png`,
      original,
      "image/png",
    );
  await goto(page, "/themes");
  await createLocal(page, "Night author");
  const saved = await save(page);
  await page.getByLabel("Name", { exact: true }).fill("Night round trip");
  await save(page);
  await openThemeAdvanced(page);
  await page.getByLabel("Package assets JSON").fill("{");
  await page
    .getByRole("button", { name: "Save complete draft package", exact: true })
    .click();
  await expect(status(page)).toContainText("Package assets must be valid JSON");
  const assetPath = "assets/pixel.png",
    manifest = {
      assets: { [assetPath]: "image/png" },
      defaults: { header: [assetPath], logo: assetPath },
      name: "Night round trip",
      schema: 1,
      style_contract: 1,
    },
    stylesheet = "[data-jaunder-theme-surface] { color: rgb(1, 2, 3); }";
  await page
    .getByLabel("theme.json", { exact: true })
    .fill(JSON.stringify(manifest));
  await page.getByLabel("style.css", { exact: true }).fill(stylesheet);
  await page
    .getByLabel("Package assets JSON")
    .fill(
      JSON.stringify([
        { path: assetPath, mime: "image/png", bytes: Array.from(original) },
      ]),
    );
  const packageSaved = response(page, "import_package");
  await page
    .getByRole("button", { name: "Save complete draft package", exact: true })
    .click();
  expect((await packageSaved).status()).toBe(200);
  await expect(status(page)).toContainText("Saved");
  const next = normal(page).getByRole("button", {
    name: "Next images",
    exact: true,
  });
  await next.click();
  await expect(
    normal(page).getByText("Image page 2", { exact: true }),
  ).toBeVisible();
  await expect(next).toBeDisabled();
  await page
    .getByLabel("Logo", { exact: true })
    .selectOption({ label: "Media: blog-logo.png" });
  await save(page);
  await openThemeAdvanced(page);
  await page.getByLabel("Header pool package asset paths").fill(assetPath);
  for (const filename of ["header-one.png", "header-two.png"]) {
    await page
      .getByLabel("Media to add", { exact: true })
      .selectOption({ label: filename });
    await page.getByRole("button", { name: "Add Media", exact: true }).click();
  }
  const pool = page.getByRole("list", { name: "Selected header Media" });
  await expect(pool).toContainText("header-one.png");
  await expect(pool).toContainText("header-two.png");
  const poolSaved = response(page, "pool");
  await page
    .getByRole("button", { name: "Save header pool", exact: true })
    .click();
  expect((await poolSaved).status()).toBe(200);
  await expect(status(page)).toContainText("Saved");
  await page.getByLabel("Header image", { exact: true }).selectOption("none");
  await expect(
    page.getByText("This replaces the rotating header images.", {
      exact: true,
    }),
  ).toBeVisible();
  await page.getByLabel("Header image", { exact: true }).selectOption("keep");
  await openThemeAdvanced(page);
  const shuffled = response(page, "shuffle");
  await page
    .getByRole("button", { name: "Shuffle assignments", exact: true })
    .click();
  expect((await shuffled).status()).toBe(200);
  await expect(status(page)).toContainText("Saved");
  const frame = await preview(page);
  await expectPreviewStructure(page, frame);
  await expect(frame.contentFrame().locator("body")).toContainText("Jaunder");
  const viewport = page.viewportSize()!;
  await page.setViewportSize({ width: 375, height: 800 });
  await expect(frame).toBeVisible();
  await page.setViewportSize(viewport);
  await page
    .getByRole("button", { name: "Use Night round trip", exact: true })
    .click();
  await expect(status(page)).toContainText("Your theme is now in use");
  await expect(current(page)).toHaveAttribute(
    "data-theme-selection",
    String(saved.id),
  );
  const context = await tracedContext();
  try {
    const p = await context.newPage();
    await goto(p, `/~${username}`);
    const image = p.locator('[data-jaunder-part="logo"]');
    await expect(image).toHaveAttribute("src", logo.url);
    const r = await p.request.get(BASE_URL + logo.url);
    expect(await r.body()).toEqual(logoBytes);
    const header = p.locator('[data-jaunder-part="header-image"]');
    await expect(header).toBeVisible();
    const url = (await header.getAttribute("src"))!;
    const expected = new Map([
      [new URL(headerOne.url, BASE_URL).pathname, oneBytes],
      [new URL(headerTwo.url, BASE_URL).pathname, twoBytes],
    ]);
    const headerBytes = url.startsWith("/theme/")
      ? original
      : expected.get(new URL(url, BASE_URL).pathname);
    expect(headerBytes).toBeDefined();
    const hr = await p.request.get(new URL(url, BASE_URL).toString());
    expect(hr.status()).toBe(200);
    expect(await hr.body()).toEqual(headerBytes);
  } finally {
    await context.close();
  }
  const fresh = await tracedContext();
  try {
    const p = await fresh.newPage();
    await signInAs(p, username);
    const release = await stallServerFn(p, "themes/list");
    try {
      await goto(p, "/themes");
      await expect(current(p)).toHaveAttribute(
        "data-theme-selection",
        String(saved.id),
      );
    } finally {
      release();
    }
    await p
      .getByRole("button", { name: "Edit Night round trip", exact: true })
      .click();
    await expect(p.getByLabel("Logo", { exact: true })).toHaveValue("keep");
    await expect(
      p.getByLabel("Logo", { exact: true }).locator('option[value="keep"]'),
    ).toHaveText("Keep current image: blog-logo.png");
    await p.getByRole("button", { name: "Next images", exact: true }).click();
    await expect(p.getByLabel("Logo", { exact: true })).toHaveValue("keep");
    await openThemeAdvanced(p);
    await expect(
      p.getByRole("list", { name: "Selected header Media" }),
    ).toContainText("header-one.png");
    await expect(
      p.getByRole("list", { name: "Selected header Media" }),
    ).toContainText("header-two.png");
  } finally {
    await fresh.close();
  }
  await openThemeAdvanced(page);
  const download = page.waitForEvent("download");
  await page.getByRole("button", { name: "Export ZIP", exact: true }).click();
  const exported = await download;
  await expect(status(page)).toContainText("download started");
  const path = (await exported.path())!;
  expect([...packageMemberDigests(path).keys()]).toEqual([
    assetPath,
    "style.css",
    "theme.json",
  ]);
  expect(packageMember(path, "theme.json")).toEqual(
    Buffer.from(JSON.stringify(manifest)),
  );
  expect(packageMember(path, "style.css")).toEqual(Buffer.from(stylesheet));
  expect(packageMember(path, assetPath)).toEqual(original);
  const removed = response(page, "remove");
  await page.getByRole("button", { name: "Delete theme", exact: true }).click();
  expect((await removed).status()).toBe(200);
  await expect(
    page.getByRole("button", { name: "Edit Night round trip", exact: true }),
  ).toHaveCount(0);
  await expect(current(page)).toHaveAttribute(
    "data-theme-selection",
    "inherit",
  );
  const imported = response(page, "import_zip");
  await page.getByLabel("Theme ZIP file").setInputFiles(path);
  expect((await imported).status()).toBe(200);
  await expect(
    page.getByRole("button", { name: "Edit Night round trip", exact: true }),
  ).toHaveAttribute("aria-expanded", "true");
  await expect(page.getByLabel("Logo", { exact: true })).toHaveValue("keep");
  await expect(
    page.getByLabel("Logo", { exact: true }).locator('option[value="keep"]'),
  ).toHaveText("Keep theme default");
  await openThemeAdvanced(page);
  await expect(
    page
      .getByRole("list", { name: "Selected header Media" })
      .getByRole("listitem"),
  ).toHaveCount(0);
});

test("published version choices stay truthful across inheritance and bundled recovery", async ({
  page,
  tracedContext,
}) => {
  const username = await signInAsNewUser(page),
    id = String(await publishTheme(page));
  await goto(page, "/themes");
  const saved = page.getByLabel("Use published version", { exact: true });
  await expect(saved).toBeHidden();
  await openThemeAdvanced(page);
  await expect(saved).toHaveValue("");
  await expect(saved.locator("option:checked")).toHaveText(
    "Choose a ready saved theme",
  );
  const writes: string[] = [];
  page.on("request", (r) => {
    const p = new URL(r.url()).pathname;
    if ([THEME_ENDPOINTS.publish, THEME_ENDPOINTS.select].includes(p))
      writes.push(p);
  });
  await saved.selectOption(id);
  await expect(current(page)).toHaveAttribute("data-theme-selection", id);
  await page.getByRole("button", { name: "Use reader", exact: true }).click();
  await expect(current(page)).toHaveText("reader");
  await expect(saved).toHaveValue("");
  await page
    .getByRole("button", { name: "Use site theme", exact: true })
    .click();
  await expect(current(page)).toHaveText("Using the site theme");
  await expect(saved).toHaveValue("");
  await saved.selectOption(id);
  await expect(saved).toHaveValue(id);
  expect(writes).toEqual(Array(4).fill(THEME_ENDPOINTS.select));
  const context = await tracedContext();
  try {
    const p = await context.newPage();
    await signInAs(p, username);
    const release = await stallServerFn(p, "themes/list");
    try {
      await goto(p, "/themes");
      await expect(
        p.getByLabel("Use published version", { exact: true }),
      ).toHaveCount(0);
    } finally {
      release();
    }
    await openThemeAdvanced(p);
    await expect(
      p.getByLabel("Use published version", { exact: true }),
    ).toHaveValue(id);
  } finally {
    await context.close();
  }
});

test("cold Studio return retains custom author selection before bundled recovery", async ({
  page,
  tracedContext,
}) => {
  const username = await signInAsNewUser(page);
  const before = await tracedContext();
  let baseline: Awaited<ReturnType<typeof publicThemeIdentity>>;
  try {
    const p = await before.newPage();
    await goto(p, `/~${username}`);
    baseline = await publicThemeIdentity(p);
  } finally {
    await before.close();
  }
  const id = await publishTheme(page);
  await goto(page, "/themes");
  await openThemeAdvanced(page);
  await page
    .getByLabel("Use published version", { exact: true })
    .selectOption(String(id));
  await expect(current(page)).toHaveAttribute(
    "data-theme-selection",
    String(id),
  );
  const context = await tracedContext();
  try {
    const p = await context.newPage();
    await signInAs(p, username);
    await goto(p, "/themes");
    await expect(current(p)).toHaveAttribute(
      "data-theme-selection",
      String(id),
    );
    await openThemeAdvanced(p);
    await expect(
      p.getByLabel("Use published version", { exact: true }),
    ).toHaveValue(String(id));
    const writes: string[] = [];
    p.on("request", (r) => {
      const path = new URL(r.url()).pathname;
      if ([THEME_ENDPOINTS.publish, THEME_ENDPOINTS.select].includes(path))
        writes.push(path);
    });
    await p.getByRole("button", { name: "Use studio", exact: true }).click();
    await expect(current(p)).toHaveAttribute("data-theme-selection", "studio");
    await p
      .getByRole("button", { name: "Use site theme", exact: true })
      .click();
    await expect(current(p)).toHaveAttribute("data-theme-selection", "inherit");
    expect(writes).toEqual([THEME_ENDPOINTS.select, THEME_ENDPOINTS.select]);
  } finally {
    await context.close();
  }
  const after = await tracedContext();
  try {
    const p = await after.newPage();
    await goto(p, `/~${username}`);
    expect(await publicThemeIdentity(p)).toEqual(baseline!);
  } finally {
    await after.close();
  }
});

test("operator manages Site catalog with confirmed writes and guarded selection restoration", async ({
  page,
  tracedContext,
}) => {
  await withSiteThemeSelection(tracedContext, async (mutate) => {
    await signInAs(page, "testoperator");
    await goto(page, "/themes");
    const site = page.getByRole("tab", { name: "Site", exact: true }),
      user = page.getByRole("tab", { name: "User", exact: true });
    await site.click();
    await expect(site).toHaveAttribute("aria-selected", "true");
    await expect(
      page.getByRole("button", { name: "Use site theme", exact: true }),
    ).toHaveCount(0);
    await createLocal(
      page,
      "Operator site published",
      ":root { outline: 2px solid rgb(4, 5, 6); }",
    );
    const saved = await save(page);
    expect(saved.applied).toBe(false);
    const frame = await preview(page);
    await expectPreviewStructure(page, frame);
    const writes: { path: string; body: string }[] = [];
    page.on("request", (r) => {
      const p = new URL(r.url()).pathname;
      if ([THEME_ENDPOINTS.publish, THEME_ENDPOINTS.select].includes(p))
        writes.push({ path: p, body: r.postData() ?? "" });
    });
    const releasePublication = await stallServerFn(page, "themes/publish"),
      releaseSelection = await stallServerFn(page, "themes/select"),
      use = page.getByRole("button", {
        name: "Use Operator site published",
        exact: true,
      });
    await mutate(async () => {
      const published = response(page, "publish"),
        selected = response(page, "select");
      try {
        await use.click();
        await expect(status(page)).toContainText("Getting your theme ready");
        for (const c of [user, site, use]) await expect(c).toBeDisabled();
        for (const c of [use, user])
          await c.evaluate((n) =>
            n.dispatchEvent(new MouseEvent("click", { bubbles: true })),
          );
        expect(writes.map((w) => w.path)).toEqual([THEME_ENDPOINTS.publish]);
        await expect(site).toHaveAttribute("aria-selected", "true");
        releasePublication();
        await expect(status(page)).toContainText("Applying your theme");
        await expect(use).toBeDisabled();
        await use.evaluate((n) =>
          n.dispatchEvent(new MouseEvent("click", { bubbles: true })),
        );
      } finally {
        releasePublication();
        releaseSelection();
      }
      const pr = await published,
        sr = await selected;
      expect(pr.status()).toBe(200);
      expect(sr.status()).toBe(200);
      confirmedMutation(
        (await pr.json()) as MutationOutcome<null>,
        "themes::publish",
      );
      confirmedMutation(
        (await sr.json()) as MutationOutcome<null>,
        "themes::select",
      );
      await expect(status(page)).toContainText("Your theme is now in use");
    });
    expect(writes.map((w) => w.path)).toEqual([
      THEME_ENDPOINTS.publish,
      THEME_ENDPOINTS.select,
    ]);
    expect(new URLSearchParams(writes[0].body).get("scope")).toBe("site");
    expect(new URLSearchParams(writes[1].body).get("selection[value]")).toBe(
      String(saved.id),
    );
    await expect(current(page)).toHaveAttribute(
      "data-theme-selection",
      String(saved.id),
    );
    const context = await tracedContext();
    try {
      for (const url of ["/", "/~testlogin"]) {
        const p = await context.newPage();
        await goto(p, url);
        await expect(p.locator(".j-root")).toHaveAttribute(
          "data-theme",
          "custom",
        );
        await expect(p.locator("[data-jaunder-theme-surface]")).toHaveCSS(
          "outline-color",
          "rgb(4, 5, 6)",
        );
        await expect(
          p.getByRole("tablist", { name: "Theme setting" }),
        ).toHaveCount(0);
      }
    } finally {
      await context.close();
    }
    await openThemeAdvanced(page);
    await mutate(async () => {
      const pending = response(page, "remove");
      await page
        .getByRole("button", { name: "Delete theme", exact: true })
        .click();
      const r = await pending;
      expect(r.status()).toBe(200);
      confirmedMutation(
        (await r.json()) as MutationOutcome<null>,
        "themes::remove",
      );
    });
    await expect(
      page.getByRole("button", {
        name: "Edit Operator site published",
        exact: true,
      }),
    ).toHaveCount(0);
    await expect(current(page)).toHaveAttribute(
      "data-theme-selection",
      "studio",
    );
    const fallback = await tracedContext();
    try {
      const p = await fallback.newPage();
      await goto(p, "/");
      await expect(p.locator(".j-root")).toHaveAttribute(
        "data-theme",
        "studio",
      );
      await expect(
        p.locator("link[data-jaunder-theme-stylesheet]"),
      ).toHaveAttribute("href", /^\/theme\/[0-9a-f]{64}$/);
    } finally {
      await fallback.close();
    }
  });
});
