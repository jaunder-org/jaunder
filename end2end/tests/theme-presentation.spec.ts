import { test, expect } from "./fixtures";
import { goto, signInAsNewUser } from "./helpers";
import { createPostViaApi } from "./posts";
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
