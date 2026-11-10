import { test, expect, slowBrowserFirstNavigationTimeoutMs } from "./fixtures";
import { goto, click, signInAsNewUser, stallServerFn } from "./helpers";
import { createPostViaApi } from "./posts";
import { navigateInApp } from "./navigate";
import { SEL } from "./selectors";

test.use({ timezoneId: "America/New_York" });

test("native Intl device zone is sent on browser creation", async ({
  page,
}, testInfo) => {
  await signInAsNewUser(page);
  await goto(page, "/posts/new", {
    timeout: slowBrowserFirstNavigationTimeoutMs(testInfo, 30_000),
  });
  await page.locator(SEL.postBody).fill("Native device timezone");
  await expect(page.locator(SEL.publishButton("false"))).toBeEnabled();
  const request = page.waitForRequest((request) =>
    request.url().endsWith("/api/posts/create"),
  );
  await click(page, SEL.publishButton("false"));
  expect((await request).postDataJSON().post.timezone).toBe("America/New_York");
  await expect(page.locator(SEL.saveSummary)).toContainText("Draft saved.");
  await expect(
    page.locator('[data-test="editing-timezone-warning"]'),
  ).toHaveCount(0);
});

test("device capture failures infer the exact device offset rather than API omission", async ({
  page,
}, testInfo) => {
  await signInAsNewUser(page);
  await goto(page, "/posts/new", {
    timeout: slowBrowserFirstNavigationTimeoutMs(testInfo, 30_000),
  });
  for (const value of [null, "", " ", "+05:00", "Etc/Unknown", "throws"]) {
    await page.evaluate(() => {
      Date.prototype.getTimezoneOffset = () => 300;
    });
    await page.evaluate((timeZone) => {
      Intl.DateTimeFormat.prototype.resolvedOptions = function () {
        if (timeZone === "throws")
          throw new Error("device timezone unavailable");
        return { timeZone } as Intl.ResolvedDateTimeFormatOptions;
      };
    }, value);
    await page.locator(SEL.postBody).fill(`Capture failure ${value}`);
    await expect(page.locator(SEL.publishButton("false"))).toBeEnabled();
    const request = page.waitForRequest((request) =>
      request.url().endsWith("/api/posts/create"),
    );
    const response = page.waitForResponse((response) =>
      response.url().endsWith("/api/posts/create"),
    );
    await click(page, SEL.publishButton("false"));
    expect((await request).postDataJSON().post.timezone).toBe("Etc/GMT+5");
    expect((await response).ok()).toBeTruthy();
    await expect(
      page.locator('[data-test="editing-timezone-warning"]'),
    ).toContainText("Inferred fixed offset Etc/GMT+5");
    await expect(page.locator(SEL.saveSummary)).toContainText("Draft saved.");
  }
});

test.describe("actual Date offset without an Intl name", () => {
  test.use({ timezoneId: "Etc/GMT+5" });

  test("throwing Intl still publishes the actual current Date offset", async ({
    page,
  }, testInfo) => {
    await signInAsNewUser(page);
    await goto(page, "/posts/new", {
      timeout: slowBrowserFirstNavigationTimeoutMs(testInfo, 30_000),
    });
    await page.evaluate(() => {
      Intl.DateTimeFormat = function () {
        throw new Error("Intl unavailable");
      } as unknown as typeof Intl.DateTimeFormat;
    });
    await page.locator(SEL.postBody).fill("Actual device Date offset");
    const request = page.waitForRequest((r) =>
      r.url().endsWith("/api/posts/create"),
    );
    const response = page.waitForResponse((r) =>
      r.url().endsWith("/api/posts/create"),
    );
    await click(page, SEL.publishButton("false"));
    expect((await request).postDataJSON().post.timezone).toBe("Etc/GMT+5");
    expect((await response).ok()).toBeTruthy();
    await expect(page.getByRole("alert")).toContainText(
      "Inferred fixed offset Etc/GMT+5",
    );
    await expect(page.locator(SEL.saveSummary)).toContainText("Draft saved.");
  });
});

test("offset fallback is exact, bounded, signed and visibly distinguished from UTC", async ({
  page,
}, testInfo) => {
  await signInAsNewUser(page);
  await goto(page, "/posts/new", {
    timeout: slowBrowserFirstNavigationTimeoutMs(testInfo, 30_000),
  });
  // JS reports minutes WEST of UTC. Literals exercise both inclusive bounds,
  // zero's supported Etc/GMT convention, and values that must never be rounded.
  for (const [west, expected] of [
    [720, "Etc/GMT+12"],
    [-720, "Etc/GMT-12"],
    [-60, "Etc/GMT-1"],
    [0, "Etc/GMT"],
    [330, "UTC"],
    [-345, "UTC"],
    [60.5, "UTC"],
    [780, "UTC"],
    [-780, "UTC"],
    [null, "UTC"],
    ["throws", "UTC"],
  ] as const) {
    await page.evaluate((west) => {
      Intl.DateTimeFormat.prototype.resolvedOptions = function () {
        throw new Error("no named zone");
      };
      Date.prototype.getTimezoneOffset = function () {
        if (west === "throws") throw new Error("no offset");
        return west === null ? NaN : west;
      };
    }, west);
    await page.locator(SEL.postBody).fill(`Exact offset ${west}`);
    const request = page.waitForRequest((r) =>
      r.url().endsWith("/api/posts/create"),
    );
    const response = page.waitForResponse((r) =>
      r.url().endsWith("/api/posts/create"),
    );
    await click(page, SEL.publishButton("false"));
    expect((await request).postDataJSON().post.timezone).toBe(expected);
    expect((await response).ok()).toBeTruthy();
    await expect(page.getByRole("alert")).toContainText(
      expected === "UTC" ? "UTC was used" : `Inferred fixed offset ${expected}`,
    );
    await expect(page.locator(SEL.saveSummary)).toContainText("Draft saved.");
  }
});

test("a published edit keeps its UTC fallback warning after navigation", async ({
  page,
}, testInfo) => {
  await signInAsNewUser(page);
  const post = await createPostViaApi(page, {
    body: "Published timezone warning",
  });
  await goto(page, `/posts/${post.post_id}/edit`, {
    timeout: slowBrowserFirstNavigationTimeoutMs(testInfo, 30_000),
  });
  await expect(page.locator(SEL.publishButton("true"))).toBeEnabled();
  await page.evaluate(() => {
    Date.prototype.getTimezoneOffset = () => NaN;
    Intl.DateTimeFormat.prototype.resolvedOptions = function () {
      throw new Error("device capture unavailable");
    };
  });
  const request = page.waitForRequest((request) =>
    request.url().endsWith("/api/posts/update"),
  );
  await navigateInApp(page, () => click(page, SEL.publishButton("true")), {
    url: post.permalink,
    ready: `#j-post-${post.post_id}`,
  });
  expect((await request).postDataJSON().post.timezone).toBe("UTC");
  await expect(
    page.locator('[data-test="editing-timezone-warning"]'),
  ).toContainText("UTC was used");
});

for (const surface of ["create", "edit", "home"] as const) {
  test(`editing timezone fallback is visible in ${surface}`, async ({
    page,
  }, testInfo) => {
    await page.setViewportSize({ width: 1280, height: 900 });
    await signInAsNewUser(page);
    let path = surface === "home" ? "/app" : "/posts/new";
    if (surface === "edit") {
      const post = await createPostViaApi(page, {
        body: "Timezone warning proof",
        publish: false,
      });
      path = `/posts/${post.post_id}/edit`;
    }
    await goto(page, path, {
      timeout: slowBrowserFirstNavigationTimeoutMs(testInfo, 30_000),
    });
    await page.locator(SEL.postBody).fill("Timezone warning proof");
    await expect(page.locator(SEL.publishButton("false"))).toBeEnabled();
    await page.screenshot({
      path: testInfo.outputPath(`${surface}-before.png`),
    });
    await page.evaluate(() => {
      Date.prototype.getTimezoneOffset = () => 300;
      const original = Intl.DateTimeFormat.prototype.resolvedOptions;
      Intl.DateTimeFormat.prototype.resolvedOptions = function () {
        return { ...original.call(this), timeZone: "No/Such_Zone" };
      };
    });
    const endpoint = surface === "edit" ? "posts/update" : "posts/create";
    const release = await stallServerFn(page, endpoint);
    const request = page.waitForRequest((request) =>
      request.url().endsWith(`/api/${endpoint}`),
    );
    await click(page, SEL.publishButton("false"));
    try {
      const payload = (await request).postDataJSON();
      expect(payload.post.timezone).toBe("Etc/GMT+5");
      await expect(page.getByRole("alert")).toContainText(
        "Inferred fixed offset Etc/GMT+5",
      );
      await page.screenshot({
        path: testInfo.outputPath(`${surface}-after.png`),
      });
    } finally {
      release();
    }
    if (surface === "home") {
      await expect(
        page.getByRole("link", { name: "Draft saved!" }),
      ).toBeVisible();
    } else {
      await expect(page.locator(SEL.saveSummary)).toContainText("Draft saved.");
    }
    await expect(page.getByRole("alert")).toContainText(
      "Inferred fixed offset Etc/GMT+5",
    );
    for (const zone of ["US/Eastern", "Europe/London"]) {
      await page.evaluate((timeZone) => {
        const original = Intl.DateTimeFormat.prototype.resolvedOptions;
        Intl.DateTimeFormat.prototype.resolvedOptions = function () {
          return { ...original.call(this), timeZone };
        };
      }, zone);
      await page.locator(SEL.postBody).fill(`Timezone capture ${zone}`);
      const request = page.waitForRequest((request) =>
        request.url().endsWith(`/api/${endpoint}`),
      );
      const response = page.waitForResponse((response) =>
        response.url().endsWith(`/api/${endpoint}`),
      );
      await click(page, SEL.publishButton("false"));
      expect((await request).postDataJSON().post.timezone).toBe(zone);
      expect((await response).ok()).toBeTruthy();
      await expect(
        page.locator('[data-test="editing-timezone-warning"]'),
      ).toHaveCount(0);
      if (surface === "home") {
        await expect(
          page.getByRole("link", { name: "Draft saved!" }),
        ).toBeVisible();
      } else {
        await expect(page.locator(SEL.saveSummary)).toContainText(
          "Draft saved.",
        );
      }
    }
  });
}
