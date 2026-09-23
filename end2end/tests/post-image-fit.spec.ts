import type { Page } from "@playwright/test";
import { test, expect } from "./fixtures";
import { goto, signInAsNewUser } from "./helpers";
import { uploadMedia } from "./media-helpers";
import { createPostViaApi } from "./posts";
import {
  conformanceThemePackage,
  publishAndSelectTheme,
} from "./theme-helpers";

const BODY = '[data-jaunder-part="post-body"]';

function imageBytes(width: number, height: number): Buffer {
  return Buffer.from(
    `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}"><rect width="${width}" height="${height}" fill="#386d89"/></svg>`,
  );
}

async function imageGeometry(page: Page, alt: string) {
  const image = page.locator(`${BODY} img[alt="${alt}"]`);
  await expect(image).toBeVisible();
  return image.evaluate(async (img: HTMLImageElement) => {
    await img.decode();
    const body = img.closest('[data-jaunder-part="post-body"]')!;
    const rect = img.getBoundingClientRect();
    const bodyStyle = getComputedStyle(body);
    return {
      naturalWidth: img.naturalWidth,
      naturalHeight: img.naturalHeight,
      width: rect.width,
      height: rect.height,
      bodyWidth:
        body.getBoundingClientRect().width -
        Number.parseFloat(bodyStyle.paddingLeft) -
        Number.parseFloat(bodyStyle.paddingRight),
      postOverflow:
        img.closest('[data-jaunder-part="post"]')!.scrollWidth -
        img.closest('[data-jaunder-part="post"]')!.clientWidth,
      documentOverflow:
        document.documentElement.scrollWidth -
        document.documentElement.clientWidth,
    };
  });
}

test("oversized Post images fit the Post body while small images remain natural", async ({
  page,
  tracedContext,
  firstNav,
}) => {
  const username = await signInAsNewUser(page);
  const wide = await uploadMedia(
    page,
    "wide.svg",
    imageBytes(1200, 800),
    "image/svg+xml",
  );
  const small = await uploadMedia(
    page,
    "small.svg",
    imageBytes(80, 50),
    "image/svg+xml",
  );
  const post = await createPostViaApi(page, {
    body: `# Image fit proof\n\n![Wide](${wide.url})\n\n![Small](${small.url})`,
  });
  const context = await tracedContext();
  try {
    for (const route of ["/", `/~${username}`, post.permalink]) {
      for (const width of [390, 1280]) {
        const publicPage = await context.newPage();
        try {
          await publicPage.setViewportSize({ width, height: 844 });
          await goto(publicPage, route, { timeout: firstNav });
          const wideGeometry = await imageGeometry(publicPage, "Wide");
          expect(wideGeometry.naturalWidth).toBe(1200);
          expect(wideGeometry.width).toBeLessThanOrEqual(
            wideGeometry.bodyWidth + 1,
          );
          expect(wideGeometry.width / wideGeometry.height).toBeCloseTo(1.5, 2);
          expect(wideGeometry.postOverflow).toBeLessThanOrEqual(1);
          expect(wideGeometry.documentOverflow).toBeLessThanOrEqual(1);
          const smallGeometry = await imageGeometry(publicPage, "Small");
          expect(smallGeometry.naturalWidth).toBe(80);
          expect(smallGeometry.width).toBe(80);
        } finally {
          await publicPage.close();
        }
      }
    }
  } finally {
    await context.close();
  }
});

test("authored dimensions retain natural proportions under a custom Theme Package", async ({
  page,
  tracedContext,
  firstNav,
}) => {
  const username = await signInAsNewUser(page);
  const wide = await uploadMedia(
    page,
    "authored.svg",
    imageBytes(1200, 800),
    "image/svg+xml",
  );
  const post = await createPostViaApi(page, {
    body: `<p>Authored image fit proof</p><img src="${wide.url}" alt="Authored" width="1000" height="200">`,
    format: "html",
  });
  const theme = conformanceThemePackage();
  await publishAndSelectTheme(page, theme);

  const context = await tracedContext();
  async function atRoute(
    route: string,
    check: (publicPage: Page) => Promise<void>,
  ) {
    const publicPage = await context.newPage();
    try {
      await publicPage.setViewportSize({ width: 390, height: 844 });
      await goto(publicPage, route, { timeout: firstNav });
      await check(publicPage);
    } finally {
      await publicPage.close();
    }
  }
  try {
    for (const route of [`/~${username}`, post.permalink]) {
      await atRoute(route, async (publicPage) => {
        await expect(publicPage.locator(".j-root")).toHaveAttribute(
          "data-theme",
          "custom",
        );
        const image = publicPage.locator(`${BODY} img[alt="Authored"]`);
        await expect(image).toHaveAttribute("width", "1000");
        await expect(image).toHaveAttribute("height", "200");
        const geometry = await imageGeometry(publicPage, "Authored");
        expect(geometry.width).toBeLessThanOrEqual(geometry.bodyWidth + 1);
        expect(geometry.width / geometry.height).toBeCloseTo(1.5, 2);
        expect(geometry.documentOverflow).toBeLessThanOrEqual(1);
      });
    }
    await atRoute("/", async (publicPage) => {
      const home = await imageGeometry(publicPage, "Authored");
      expect(home.width).toBeCloseTo(home.bodyWidth, 0);
    });
  } finally {
    await context.close();
  }
});

test("Markdown, Org, and HTML image rendering shares the Post-body wrapper", async ({
  page,
  tracedContext,
  firstNav,
}) => {
  await signInAsNewUser(page);
  const media = await uploadMedia(
    page,
    "formats.svg",
    imageBytes(1200, 800),
    "image/svg+xml",
  );
  const posts = [
    await createPostViaApi(page, { body: `![Format](${media.url})` }),
    await createPostViaApi(page, { body: `[[${media.url}]]`, format: "org" }),
    await createPostViaApi(page, {
      body: `<img src="${media.url}" alt="Format">`,
      format: "html",
    }),
  ];
  const context = await tracedContext();
  try {
    for (const post of posts) {
      const publicPage = await context.newPage();
      try {
        await goto(publicPage, post.permalink, { timeout: firstNav });
        await expect(publicPage.locator(`${BODY} img`)).toHaveCount(1);
      } finally {
        await publicPage.close();
      }
    }
  } finally {
    await context.close();
  }
});

test("a public custom Theme Package can override the Post image fit default", async ({
  page,
  tracedContext,
  firstNav,
}) => {
  await signInAsNewUser(page);
  const wide = await uploadMedia(
    page,
    "override.svg",
    imageBytes(1200, 800),
    "image/svg+xml",
  );
  const post = await createPostViaApi(page, {
    body: `![Override](${wide.url})`,
  });
  const theme = conformanceThemePackage();
  theme.stylesheet +=
    '\n[data-jaunder-part="post-body"] img { max-width: 50%; }';
  await publishAndSelectTheme(page, theme);
  const context = await tracedContext();
  try {
    const publicPage = await context.newPage();
    try {
      await publicPage.setViewportSize({ width: 390, height: 844 });
      await goto(publicPage, post.permalink, { timeout: firstNav });
      const image = await imageGeometry(publicPage, "Override");
      expect(image.width).toBeCloseTo(image.bodyWidth / 2, 0);
      expect(image.width / image.height).toBeCloseTo(1.5, 2);
    } finally {
      await publicPage.close();
    }
  } finally {
    await context.close();
  }
});
