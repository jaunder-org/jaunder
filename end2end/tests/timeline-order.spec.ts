import type { Page } from "@playwright/test";
import { expect, setTestBudget, test } from "./fixtures";
import {
  BASE_URL,
  click,
  goto,
  signInAsNewUser,
  stallServerFn,
  waitForMount,
} from "./helpers";
import { createPostViaApi } from "./posts";
import { seedPostsViaTool } from "./seed";

const ORDER_CONTROL = '[data-jaunder-part="timeline-order"]';
const POST_LIST = '[data-jaunder-part="post-list"]';
const PUBLISHED_TIME = '[data-jaunder-part="published-time"]';
const PAGE_SIZE = 50;
const NEWEST_TITLES = [
  "Timeline Order Late",
  "Timeline Order Middle",
  "Timeline Order Early",
] as const;
const OLDEST_TITLES = [...NEWEST_TITLES].reverse();

async function expectOrderControl(page: Page): Promise<void> {
  const order = page.getByRole("button", {
    name: "Newest first; show oldest first",
    exact: true,
  });
  await expect(order).toHaveAttribute("data-order", "newest");
  await expect(order.locator("svg")).toHaveCount(1);
  const scroll = page
    .locator(".j-scroll")
    .filter({ has: page.locator(POST_LIST) });
  await expect(scroll).toHaveCount(1);
  const timelineParts = scroll.locator(`${ORDER_CONTROL}, ${POST_LIST}`);
  await expect(timelineParts).toHaveCount(2);
  expect(
    await timelineParts.evaluateAll((elements) =>
      elements.map((element) => element.getAttribute("data-jaunder-part")),
    ),
  ).toEqual(["timeline-order", "post-list"]);
}

async function createOrderedPosts(page: Page, tag: string): Promise<void> {
  await Promise.all([
    createPostViaApi(page, {
      body: "# Timeline Order Early\n\nEarliest ordered fixture post.",
      tags: [tag],
      publishAt: "2000-01-01T00:00:00Z",
    }),
    createPostViaApi(page, {
      body: "# Timeline Order Middle\n\nMiddle ordered fixture post.",
      tags: [tag],
      publishAt: "2001-01-01T00:00:00Z",
    }),
    createPostViaApi(page, {
      body: "# Timeline Order Late\n\nLatest ordered fixture post.",
      tags: [tag],
      publishAt: "2002-01-01T00:00:00Z",
    }),
  ]);
}

async function expectOrderedPosts(
  page: Page,
  titles: readonly string[],
): Promise<void> {
  await expect
    .poll(async () => {
      const texts = await page
        .locator(`${POST_LIST} article.j-post`)
        .allTextContents();
      const positions = titles.map((title) =>
        texts.findIndex((text) => text.includes(title)),
      );
      return positions.every(
        (position, index) =>
          position >= 0 && (index === 0 || position > positions[index - 1]!),
      );
    })
    .toBe(true);
}

async function expectPublishedTimeOrder(
  page: Page,
  direction: "ascending" | "descending",
): Promise<void> {
  await expect
    .poll(async () => {
      const times = await page
        .locator(`${POST_LIST} ${PUBLISHED_TIME}`)
        .allTextContents();
      const sorted = [...times].sort((left, right) =>
        direction === "ascending"
          ? left.localeCompare(right)
          : right.localeCompare(left),
      );
      return (
        times.length > 0 && times.every((time, index) => time === sorted[index])
      );
    })
    .toBe(true);
}

async function expectOldestSeededPosts(
  page: Page,
  prefix: string,
): Promise<void> {
  const escapedPrefix = prefix.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const suffix = new RegExp(`${escapedPrefix}(\\d+)(?=\\s|$)`);
  const expected = Array.from({ length: PAGE_SIZE + 1 }, (_, index) => index);
  await expect
    .poll(async () =>
      (
        await page.locator(`${POST_LIST} article.j-post`).allTextContents()
      ).flatMap((text) => {
        const match = text.match(suffix);
        return match ? [Number(match[1])] : [];
      }),
    )
    .toEqual(expected);
}

async function selectOldest(page: Page, path: string): Promise<void> {
  const order = page.getByRole("button", {
    name: "Newest first; show oldest first",
    exact: true,
  });
  await expect(order).toHaveAttribute("data-order", "newest");
  await click(page, `${ORDER_CONTROL} button`);
  await page.waitForURL(`${BASE_URL}${path}?order=oldest`);
  await expect(
    page.getByRole("button", {
      name: "Oldest first; show newest first",
      exact: true,
    }),
  ).toHaveAttribute("data-order", "oldest");
}

test("timeline order is URL-driven on every post timeline surface", async ({
  page,
  firstNav,
}) => {
  // Covers five mounted routes plus history restoration and a bare-URL re-entry.
  setTestBudget(60_000);
  const username = await signInAsNewUser(page);
  const tag = `timelineorder${Date.now()}`;
  await createOrderedPosts(page, tag);

  const userTagPath = `/~${username}/tags/${tag}`;
  const routes = ["/", "/app", `/~${username}`, `/tags/${tag}`, userTagPath];
  let appPage: Page | undefined;
  let userTagPage: Page | undefined;

  for (const path of routes) {
    const routePage = await page.context().newPage();
    await goto(routePage, path, { timeout: firstNav });
    await expectOrderControl(routePage);
    if (path === "/") {
      await expectPublishedTimeOrder(routePage, "descending");
    } else {
      await expectOrderedPosts(routePage, NEWEST_TITLES);
    }
    await selectOldest(routePage, path);
    if (path === "/") {
      await expectPublishedTimeOrder(routePage, "ascending");
    } else {
      await expectOrderedPosts(routePage, OLDEST_TITLES);
    }

    if (path === "/app") {
      appPage = routePage;
    } else if (path === userTagPath) {
      userTagPage = routePage;
    } else {
      await routePage.close();
    }
  }

  if (!appPage || !userTagPage) {
    throw new Error("timeline route pages were not created");
  }

  await userTagPage.goBack();
  await userTagPage.waitForURL(`${BASE_URL}${userTagPath}`);
  await expect(
    userTagPage.getByRole("button", {
      name: "Newest first; show oldest first",
      exact: true,
    }),
  ).toHaveAttribute("data-order", "newest");
  await expectOrderedPosts(userTagPage, NEWEST_TITLES);

  await userTagPage.goForward();
  await userTagPage.waitForURL(`${BASE_URL}${userTagPath}?order=oldest`);
  await expect(
    userTagPage.getByRole("button", {
      name: "Oldest first; show newest first",
      exact: true,
    }),
  ).toHaveAttribute("data-order", "oldest");
  await expectOrderedPosts(userTagPage, OLDEST_TITLES);

  const unknownPage = await page.context().newPage();
  await goto(unknownPage, `${userTagPath}?order=unknown`, {
    timeout: firstNav,
  });
  await expect(
    unknownPage.getByRole("button", {
      name: "Newest first; show oldest first",
      exact: true,
    }),
  ).toHaveAttribute("data-order", "newest");
  await expectOrderedPosts(unknownPage, NEWEST_TITLES);

  const bareAppPage = await page.context().newPage();
  await goto(bareAppPage, "/app", { timeout: firstNav });
  await expect(
    bareAppPage.getByRole("button", {
      name: "Newest first; show oldest first",
      exact: true,
    }),
  ).toHaveAttribute("data-order", "newest");

  await bareAppPage.close();
  await unknownPage.close();
  await userTagPage.close();
  await appPage.close();
});

test("Oldest timeline seeds remain ordered through CSR mount and load more", async ({
  page,
  tracedContext,
  firstNav,
}) => {
  // Covers deterministic setup, a separate projector entry, and pagination.
  setTestBudget(60_000);
  const username = await signInAsNewUser(page);
  const tag = `timelineload${Date.now()}`;
  await createOrderedPosts(page, tag);
  await seedPostsViaTool(username, PAGE_SIZE + 1, `Timeline Load ${tag}`);

  const guestContext = await tracedContext();
  const guestPage = await guestContext.newPage();
  const release = await stallServerFn(
    guestPage,
    "timeline/list_local_timeline",
  );

  // goto() waits for CSR mount. The raw entry is deliberate: while the first
  // client fetch is held, these assertions observe the public projector paint.
  await guestPage.goto(`${BASE_URL}/?order=oldest`, {
    waitUntil: "domcontentloaded",
    timeout: firstNav,
  });
  await expect(
    guestPage.getByRole("button", {
      name: "Oldest first; show newest first",
      exact: true,
    }),
  ).toHaveAttribute("data-order", "oldest");
  await expectOrderedPosts(guestPage, OLDEST_TITLES);

  release();
  await waitForMount(guestPage);
  await expect(
    guestPage.getByRole("button", {
      name: "Oldest first; show newest first",
      exact: true,
    }),
  ).toHaveAttribute("data-order", "oldest");
  await expectOrderedPosts(guestPage, OLDEST_TITLES);

  await goto(page, `/~${username}`, { timeout: firstNav });
  await selectOldest(page, `/~${username}`);
  await expect(page.locator(`${POST_LIST} article.j-post`)).toHaveCount(
    PAGE_SIZE,
  );
  await click(page, 'button:has-text("Load more")');
  await expect(page.locator(`${POST_LIST} article.j-post`)).toHaveCount(
    PAGE_SIZE + 4,
  );
  await expect(
    page.getByRole("button", {
      name: "Oldest first; show newest first",
      exact: true,
    }),
  ).toHaveAttribute("data-order", "oldest");
  await expectOldestSeededPosts(page, `Timeline Load ${tag} `);

  await guestContext.close();
});
