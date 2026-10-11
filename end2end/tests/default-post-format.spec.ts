import type { Page } from "@playwright/test";
import { test, expect, slowBrowserFirstNavigationTimeoutMs } from "./fixtures";
import {
  BASE_URL,
  click,
  confirmedMutation,
  failServerFn,
  goto,
  signInAsNewUser,
  stallServerFn,
  type MutationOutcome,
} from "./helpers";
import {
  createPostViaApi,
  openComposerControl,
  openComposerFromSidebar,
  openEditor,
} from "./posts";
import { SEL } from "./selectors";

type Format = "markdown" | "org" | "html";
type Saved = { post: { post_id: number; permalink: string } };

async function savePreference(page: Page, format: Format) {
  const saved = await page.request.post(
    `${BASE_URL}/api/profile/set_default_post_format`,
    {
      form: { format },
    },
  );
  expect(saved.ok()).toBe(true);
  confirmedMutation(
    (await saved.json()) as MutationOutcome<null>,
    "save format preference",
  );
}

async function expectStoredFormat(page: Page, postId: number, format: Format) {
  const fetched = await page.request.post(`${BASE_URL}/api/posts/get_preview`, {
    form: { post_id: String(postId) },
  });
  expect(fetched.ok()).toBe(true);
  expect((await fetched.json()).post.format).toBe(format);
}

async function publish(page: Page): Promise<number> {
  await expect(page.locator(SEL.publishButton("true"))).toBeEnabled();
  const created = page.waitForResponse((response) =>
    response.url().endsWith("/api/posts/create"),
  );
  await click(page, SEL.publishButton("true"));
  const response = await created;
  expect(response.ok()).toBe(true);
  return confirmedMutation(
    (await response.json()) as MutationOutcome<Saved>,
    "create Post",
  ).post.post_id;
}

for (const entry of ["/app", "/posts/new"]) {
  for (const [format, label] of [
    ["markdown", "Markdown"],
    ["org", "Org"],
    ["html", "HTML"],
  ] as const) {
    test(`new Posts at ${entry} use the saved ${label} default`, async ({
      page,
    }, testInfo) => {
      await signInAsNewUser(page);
      await savePreference(page, format);
      await goto(page, entry, {
        timeout: slowBrowserFirstNavigationTimeoutMs(testInfo, 15_000),
      });
      const control = page
        .locator(".j-composer-control-summary")
        .filter({ has: page.getByText("Format", { exact: true }) });
      await expect(control).toContainText(label);
      if (format !== "html") {
        await openComposerControl(page, "Format");
        await expect(page.locator(SEL.formatButton(label))).toHaveAttribute(
          "aria-pressed",
          "true",
        );
      }
      await page.locator(SEL.postBody).fill("*emphasis*");
      await expectStoredFormat(page, await publish(page), format);
    });
  }

  test(`delayed preference at ${entry} blocks submission without losing content`, async ({
    page,
  }, testInfo) => {
    await signInAsNewUser(page);
    await savePreference(page, "org");
    const release = await stallServerFn(
      page,
      "profile/get_default_post_format",
    );
    await goto(page, entry, {
      timeout: slowBrowserFirstNavigationTimeoutMs(testInfo, 15_000),
    });
    await page.locator(SEL.postBody).fill("Content entered while loading");
    await openComposerControl(page, "Format");
    await expect(page.getByRole("status")).toContainText(
      "Loading Default Post Format",
    );
    await expect(page.locator(".j-seg button[aria-pressed=true]")).toHaveCount(
      0,
    );
    await expect(page.locator(SEL.formatButton("Org"))).toBeDisabled();
    await expect(page.locator(SEL.publishButton("true"))).toBeDisabled();
    await release();
    await expect(page.locator(SEL.formatButton("Org"))).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    await expect(page.locator(SEL.postBody)).toHaveValue(
      "Content entered while loading",
    );
    await expectStoredFormat(page, await publish(page), "org");
  });

  test(`failed preference at ${entry} retries without inventing Markdown`, async ({
    page,
  }, testInfo) => {
    await signInAsNewUser(page);
    await savePreference(page, "org");
    await failServerFn(page, "profile/get_default_post_format");
    await goto(page, entry, {
      timeout: slowBrowserFirstNavigationTimeoutMs(testInfo, 15_000),
    });
    await page.locator(SEL.postBody).fill("Content survives retry");
    await openComposerControl(page, "Format");
    await expect(page.getByRole("alert")).toContainText(
      "Could not load the Default Post Format.",
    );
    await expect(page.locator(".j-seg button[aria-pressed=true]")).toHaveCount(
      0,
    );
    await expect(page.locator(SEL.publishButton("true"))).toBeDisabled();
    await page.unroute("**/api/profile/get_default_post_format");
    await page
      .getByRole("button", { name: "Retry Default Post Format" })
      .click();
    await expect(page.getByRole("alert")).toHaveCount(0);
    await expect(page.locator(SEL.formatButton("Org"))).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    await expect(page.locator(SEL.postBody)).toHaveValue(
      "Content survives retry",
    );
    await expectStoredFormat(page, await publish(page), "org");
  });
}

test("open composition owns its format across preference changes and successful submissions", async ({
  page,
}, testInfo) => {
  await signInAsNewUser(page);
  await savePreference(page, "org");
  await goto(page, "/app", {
    timeout: slowBrowserFirstNavigationTimeoutMs(testInfo, 15_000),
  });
  await openComposerControl(page, "Format");
  await expect(page.locator(SEL.formatButton("Org"))).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await savePreference(page, "html");
  await expect(page.locator(SEL.formatButton("Org"))).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await click(page, SEL.formatButton("Markdown"));
  await page.locator(SEL.postBody).fill("First explicitly Markdown Post");
  await expectStoredFormat(page, await publish(page), "markdown");
  await expect(page.locator(SEL.postBody)).toHaveValue("");
  await expect(page.locator(SEL.formatButton("Markdown"))).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await page.locator(SEL.postBody).fill("Second explicitly Markdown Post");
  await expectStoredFormat(page, await publish(page), "markdown");
  await expect(page.locator(SEL.postBody)).toHaveValue("");
  await openComposerFromSidebar(page);
  await expect(
    page
      .locator(".j-composer-control-summary")
      .filter({ has: page.getByText("Format", { exact: true }) }),
  ).toContainText("HTML");
});

test("existing Post editing retains its format independently of the preference", async ({
  page,
}, testInfo) => {
  await signInAsNewUser(page);
  const post = await createPostViaApi(page, {
    body: "*existing Org Post*\n",
    format: "org",
  });
  await savePreference(page, "markdown");
  await goto(page, post.permalink, {
    timeout: slowBrowserFirstNavigationTimeoutMs(testInfo, 15_000),
  });
  await openEditor(page);
  await openComposerControl(page, "Format");
  await expect(page.locator(SEL.formatButton("Org"))).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await expect(page.locator(SEL.postBody)).toHaveValue("*existing Org Post*\n");
});
