import { test, expect } from "./fixtures";
import { goto, signInAsNewUser } from "./helpers";
import { createPostViaApi } from "./posts";
import { navigateInApp } from "./navigate";
import {
  conformanceThemePackage,
  contrastRatio,
  publishAndSelectTheme,
} from "./theme-helpers";

async function publicThemePage(
  page: Parameters<typeof goto>[0],
  username: string,
) {
  await goto(page, `/~${username}`);
  await expect(page.locator(".j-root")).toHaveAttribute("data-theme", "custom");
  await expect(page.locator('[data-jaunder-part="post-list"]')).toBeVisible();
}

test("custom package propagates an opaque contrasting timeline divider in dark presentation", async ({
  page,
  tracedContext,
}) => {
  const username = await signInAsNewUser(page);
  await createPostViaApi(page, { body: "# Conformance divider\n\nBody" });
  await publishAndSelectTheme(page);

  const publicContext = await tracedContext();
  try {
    const publicPage = await publicContext.newPage();
    await publicThemePage(publicPage, username);
    const light = await publicPage
      .locator('[data-jaunder-part="post"]')
      .first()
      .evaluate((post) => ({
        divider: getComputedStyle(post).borderBlockEndColor,
        surface: getComputedStyle(
          document.querySelector('[data-jaunder-part="main"]')!,
        ).backgroundColor,
      }));

    await publicPage.emulateMedia({ colorScheme: "dark" });
    await expect(
      publicPage.locator('[data-jaunder-part="post"]').first(),
    ).toHaveCSS("border-block-end-color", "rgb(143, 163, 184)");
    const dark = await publicPage
      .locator('[data-jaunder-part="post"]')
      .first()
      .evaluate((post) => ({
        divider: getComputedStyle(post).borderBlockEndColor,
        surface: getComputedStyle(
          document.querySelector('[data-jaunder-part="main"]')!,
        ).backgroundColor,
      }));

    expect(dark.divider).not.toBe("rgba(0, 0, 0, 0)");
    expect(dark.divider).not.toBe(light.divider);
    // A divider is a non-text UI boundary, so its contrast with the adjacent
    // dark semantic surface follows the WCAG 3:1 UI contrast threshold.
    expect(contrastRatio(dark.divider, dark.surface)).toBeGreaterThanOrEqual(3);
  } finally {
    await publicContext.close();
  }
});

test("Theme Package overrides a public syntax token without changing Home", async ({
  page,
  tracedContext,
}) => {
  await signInAsNewUser(page);
  const post = await createPostViaApi(page, {
    body: '# Semantic token\n\n```elisp\n(message "theme")\n```',
    audience: "public",
  });
  const themePackage = conformanceThemePackage();
  themePackage.stylesheet +=
    '\n[data-jaunder-part="post-body"] { --j-syn-string: rgb(0, 90, 120); }\n';
  await publishAndSelectTheme(page, themePackage);

  const publicContext = await tracedContext();
  try {
    const publicPage = await publicContext.newPage();
    await goto(publicPage, post.permalink);
    await expect(publicPage.locator(".j-root")).toHaveAttribute(
      "data-theme",
      "custom",
    );
    await expect(
      publicPage.locator(".j-post-body pre code .j-syn-string").first(),
    ).toHaveCSS("color", "rgb(0, 90, 120)");

    await goto(page, "/app");
    const homeToken = page.locator(
      'article.j-post:has-text("Semantic token") pre code .j-syn-string',
    );
    await expect(homeToken.first()).toBeVisible();
    await expect(homeToken.first()).not.toHaveCSS("color", "rgb(0, 90, 120)");
    const home = page.locator(".j-root");
    await expect(home).not.toHaveAttribute("data-theme", "custom");
    await expect(home).toHaveAttribute("data-jaunder-private", "true");
    await expect(home).toHaveCSS("--post-pad-y", "20px");
    await expect(
      page.locator("link[data-jaunder-theme-stylesheet]"),
    ).toHaveCount(0);
    const applicationStylesheets = await page
      .locator('link[rel="stylesheet"]')
      .evaluateAll((links) => links.map((link) => link.getAttribute("href")));
    expect(applicationStylesheets).toHaveLength(1);
    expect(applicationStylesheets[0]).toMatch(/^\/theme\/[0-9a-f]{64}$/);
  } finally {
    await publicContext.close();
  }
});

test("private stylesheet cleanup fault is visible and retries through the coordinator", async ({
  page,
}) => {
  await signInAsNewUser(page);
  await page.addInitScript(() => {
    const original = Document.prototype.querySelectorAll;
    const state = { fired: false };
    Document.prototype.querySelectorAll = function (selector: string) {
      if (selector === "link[data-jaunder-theme-stylesheet]") {
        state.fired = true;
        throw new DOMException("injected private cleanup failure");
      }
      return original.call(this, selector);
    };
    Object.assign(globalThis, {
      __jaunderThemeCleanupFault: {
        state,
        restore: () => {
          Document.prototype.querySelectorAll = original;
        },
      },
    });
  });

  try {
    await goto(page, "/app");
    await expect(page.locator(".error")).toContainText(
      "Unable to clean up Theme Package stylesheet",
    );
    await expect(page.getByRole("link", { name: "Compose" })).toHaveCount(0);
    expect(
      await page.evaluate(
        () =>
          (
            globalThis as typeof globalThis & {
              __jaunderThemeCleanupFault: { state: { fired: boolean } };
            }
          ).__jaunderThemeCleanupFault.state.fired,
      ),
    ).toBe(true);

    await page.evaluate(() =>
      (
        globalThis as typeof globalThis & {
          __jaunderThemeCleanupFault: { restore: () => void };
        }
      ).__jaunderThemeCleanupFault.restore(),
    );
    await page.getByRole("button", { name: "Retry" }).click();
    await expect(page.getByRole("link", { name: "Compose" })).toBeVisible();
    await expect(page.locator(".j-root")).toHaveCSS("--post-pad-y", "20px");
    await expect(page.locator(".error")).toHaveCount(0);
    await expect(page.locator('link[rel="stylesheet"]')).toHaveCount(1);
    await expect(page.locator('link[rel="stylesheet"]')).toHaveAttribute(
      "href",
      /^\/theme\/[0-9a-f]{64}$/,
    );
    await expect(page.locator("link[data-jaunder-theme-staged]")).toHaveCount(
      0,
    );
    await expect(
      page.locator("link[data-jaunder-theme-stylesheet]"),
    ).toHaveCount(0);
  } finally {
    await page.evaluate(() =>
      (
        globalThis as typeof globalThis & {
          __jaunderThemeCleanupFault?: { restore: () => void };
        }
      ).__jaunderThemeCleanupFault?.restore(),
    );
  }
});

for (const fault of ["promotion", "append"] as const) {
  test(`Theme Package ${fault} failure retains the previous public stylesheet`, async ({
    page: owner,
    tracedContext,
  }) => {
    await signInAsNewUser(owner);
    const post = await createPostViaApi(owner, {
      body: "# Staged package failure",
      audience: "public",
    });
    await publishAndSelectTheme(owner);
    const publicContext = await tracedContext();
    const page = await publicContext.newPage();
    try {
      await goto(page, "/");
      const active = page.locator("link[data-jaunder-theme-stylesheet]");
      await expect(page.locator(".j-root")).toHaveAttribute(
        "data-theme",
        "studio",
      );
      await expect(active).toHaveCount(1);
      const oldHref = await active.getAttribute("href");
      await page.evaluate((fault) => {
        const attribute = Element.prototype.setAttribute;
        const append = Node.prototype.appendChild;
        const state = { fired: false };
        Element.prototype.setAttribute = function (
          name: string,
          value: string,
        ) {
          if (
            fault === "promotion" &&
            this instanceof HTMLLinkElement &&
            name === "data-jaunder-theme-stylesheet" &&
            this.media === "not all"
          ) {
            state.fired = true;
            throw new DOMException("injected Theme Package promotion failure");
          }
          return attribute.call(this, name, value);
        };
        Node.prototype.appendChild = function <T extends Node>(node: T): T {
          if (
            fault === "append" &&
            node instanceof HTMLLinkElement &&
            node.hasAttribute("data-jaunder-theme-staged")
          ) {
            state.fired = true;
            throw new DOMException("injected Theme Package append failure");
          }
          return append.call(this, node) as T;
        };
        Object.assign(globalThis, {
          __jaunderPromotionFault: {
            state,
            restore: () => {
              Element.prototype.setAttribute = attribute;
              Node.prototype.appendChild = append;
            },
          },
        });
      }, fault);
      await navigateInApp(
        page,
        () => page.locator(`a[href="${post.permalink}"]`).first().click(),
        { url: post.permalink, ready: ".error" },
      );
      await expect(page.locator(".error")).toContainText(
        fault === "promotion"
          ? "Unable to promote Theme Package stylesheet"
          : "Unable to stage Theme Package stylesheet",
      );
      expect(
        await page.evaluate(
          () =>
            (
              globalThis as typeof globalThis & {
                __jaunderPromotionFault: { state: { fired: boolean } };
              }
            ).__jaunderPromotionFault.state.fired,
        ),
      ).toBe(true);
      await expect(active).toHaveCount(1);
      await expect(active).toHaveAttribute("href", oldHref!);
      await expect(page.locator(".j-root")).toHaveAttribute(
        "data-theme",
        "studio",
      );
      await expect(page.locator("link[data-jaunder-theme-staged]")).toHaveCount(
        0,
      );
      expect(
        await active.evaluate(
          (link) => (link as HTMLLinkElement).sheet !== null,
        ),
      ).toBe(true);
    } finally {
      await page.evaluate(() =>
        (
          globalThis as typeof globalThis & {
            __jaunderPromotionFault?: { restore: () => void };
          }
        ).__jaunderPromotionFault?.restore(),
      );
      await publicContext.close();
    }
  });
}

test("a late bundled stylesheet cannot overwrite a newer public presentation", async ({
  page: owner,
  tracedContext,
}) => {
  await signInAsNewUser(owner);
  const post = await createPostViaApi(owner, {
    body: "# Cancellation terminal post\n\nA real public destination.",
    audience: "public",
  });
  await goto(owner, "/themes");
  const selection = owner.getByLabel("Public selection");
  await selection.selectOption("terminal");
  await expect(selection).toHaveValue("terminal");

  const anonymousContext = await tracedContext();
  const page = await anonymousContext.newPage();
  let release!: () => void;
  let fired!: () => void;
  let settled!: () => void;
  const releaseGate = new Promise<void>((resolve) => {
    release = resolve;
  });
  const requestFired = new Promise<void>((resolve) => {
    fired = resolve;
  });
  const requestSettled = new Promise<void>((resolve) => {
    settled = resolve;
  });
  let heldStylesheet: string | undefined;
  try {
    await goto(page, "/");
    const studioHref = await page
      .locator("link[data-jaunder-theme-stylesheet]")
      .getAttribute("href");
    expect(studioHref).toMatch(/^\/theme\/[0-9a-f]{64}$/);

    await page.route("**/theme/*", async (route) => {
      const request = route.request();
      if (
        heldStylesheet === undefined &&
        request.resourceType() === "stylesheet" &&
        new URL(request.url()).pathname !== studioHref
      ) {
        heldStylesheet = new URL(request.url()).pathname;
        fired();
        await releaseGate;
        try {
          await route.continue();
        } finally {
          settled();
        }
        return;
      }
      await route.continue();
    });

    await navigateInApp(
      page,
      () =>
        page.getByRole("link", { name: "Cancellation terminal post" }).click(),
      { url: post.permalink, ready: ".j-loading" },
    );
    await requestFired;
    const staged = page.locator("link[data-jaunder-theme-staged]");
    await expect(staged).toHaveCount(1);
    await expect(staged).toHaveAttribute("href", heldStylesheet!);
    await expect(staged).toHaveAttribute("media", "not all");
    const cancelledLink = await staged.elementHandle();
    expect(cancelledLink).not.toBeNull();

    await navigateInApp(
      page,
      () => page.getByRole("link", { name: "Local" }).click(),
      { url: "/", ready: '[data-jaunder-part="post-list"]' },
    );
    await expect(page.locator(".j-root")).toHaveAttribute(
      "data-theme",
      "studio",
    );
    await expect(
      page.locator("link[data-jaunder-theme-stylesheet]"),
    ).toHaveAttribute("href", studioHref!);

    expect(
      await cancelledLink!.evaluate((node) => {
        const link = node as HTMLLinkElement;
        return (
          !link.isConnected && link.onload === null && link.onerror === null
        );
      }),
    ).toBe(true);
    release();
    await requestSettled;
    // The real request is released; a detached link also has no callback through
    // which a late browser load notification could promote the old destination.
    await cancelledLink!.evaluate((node) =>
      node.dispatchEvent(new Event("load")),
    );
    await expect(page).toHaveURL(/\/$/);
    await expect(page.locator(".j-root")).toHaveAttribute(
      "data-theme",
      "studio",
    );
    await expect(
      page.locator("link[data-jaunder-theme-stylesheet]"),
    ).toHaveCount(1);
    await expect(
      page.locator("link[data-jaunder-theme-stylesheet]"),
    ).toHaveAttribute("href", studioHref!);
    await expect(page.locator("link[data-jaunder-theme-staged]")).toHaveCount(
      0,
    );
  } finally {
    release?.();
    if (heldStylesheet !== undefined) await requestSettled;
    await page.unroute("**/theme/*");
    await anonymousContext.close();
  }
});

test("Theme Package without syntax hooks inherits readable token defaults", async ({
  page,
  tracedContext,
}) => {
  await signInAsNewUser(page);
  const post = await createPostViaApi(page, {
    body: '# Legacy syntax\n\n```elisp\n(message "legacy")\n```',
    audience: "public",
  });
  await publishAndSelectTheme(page, conformanceThemePackage());
  const publicContext = await tracedContext();
  try {
    const publicPage = await publicContext.newPage();
    await goto(publicPage, post.permalink);
    await expect(publicPage.locator(".j-root")).toHaveAttribute(
      "data-theme",
      "custom",
    );
    const token = publicPage
      .locator(".j-post-body pre code .j-syn-string")
      .first();
    await expect(token).toBeVisible();
    const colors = await token.evaluate((element) => ({
      token: getComputedStyle(element).color,
      plain: getComputedStyle(element.closest("code")!).color,
      surface: getComputedStyle(element.closest("pre")!).backgroundColor,
    }));
    expect(colors.token).not.toBe(colors.plain);
    expect(contrastRatio(colors.token, colors.surface)).toBeGreaterThanOrEqual(
      4.5,
    );
  } finally {
    await publicContext.close();
  }
});

test("custom package font compilation and immutable asset serving retain byte identity", async ({
  page,
  tracedContext,
}) => {
  const username = await signInAsNewUser(page);
  await createPostViaApi(page, {
    body: "# Conformance font\n\nRepresentative body",
  });
  const themePackage = conformanceThemePackage();
  await publishAndSelectTheme(page, themePackage);

  const publicContext = await tracedContext();
  try {
    const publicPage = await publicContext.newPage();
    await publicThemePage(publicPage, username);
    const stylesheetHref = await publicPage
      .locator("link[data-jaunder-theme-stylesheet]")
      .getAttribute("href");
    expect(stylesheetHref).toMatch(/^\/theme\/[0-9a-f]{64}$/);
    const stylesheet = await publicPage.request.get(
      new URL(stylesheetHref!, publicPage.url()).toString(),
    );
    expect(stylesheet.status()).toBe(200);
    const css = await stylesheet.text();
    expect(css).toMatch(/jaunder-[0-9a-f]+-Conformance Sans/);

    const fontUrl = /\/theme\/[0-9a-f]{64}/g.exec(css)?.[0];
    expect(
      fontUrl,
      "compiled CSS must reference an immutable font asset",
    ).toBeTruthy();
    const font = await publicPage.request.get(
      new URL(fontUrl!, publicPage.url()).toString(),
    );
    expect(font.status()).toBe(200);
    expect(await font.body()).toEqual(
      Buffer.from(themePackage.assets[0].bytes),
    );

    const fontProbe = await publicPage
      .locator('[data-jaunder-part="post-body"]')
      .evaluate(async (body) => {
        const family = getComputedStyle(body).fontFamily;
        const namespacedFamily = family.match(
          /jaunder-[0-9a-f]+-Conformance Sans/,
        )?.[0];
        if (namespacedFamily === undefined) return { family, loaded: false };
        const descriptor = `16px "${namespacedFamily}"`;
        await document.fonts.load(descriptor);
        await document.fonts.ready;
        return { family, loaded: document.fonts.check(descriptor) };
      });
    expect(fontProbe.family).toMatch(/jaunder-[0-9a-f]+-Conformance Sans/);
    expect(fontProbe.loaded).toBe(true);
  } finally {
    await publicContext.close();
  }
});
